//! Community plugin system (v1.0) — sandboxed, data-only WebAssembly.
//!
//! Per the build plan: built-in adapters are native Rust; community plugins are
//! **WASM and pure-data**. A plugin cannot spawn processes, read the filesystem,
//! or reach the network — this runtime links *no* host imports, so the only
//! thing a plugin can do is transform bytes we hand it inside its own linear
//! memory. Execution is fuel-metered, so a malicious or buggy plugin can't hang
//! the engine in an infinite loop.
//!
//! Plugins live at `<data_dir>/plugins/<id>/` with a `plugin.json` manifest and
//! a `.wasm` module. They are **disabled until the user opts in** (a clear trust
//! gate, even though the sandbox is the real protection); enablement is stored
//! in the app config.
//!
//! ## Plugin ABI (v1)
//! The wasm module must export:
//! - `memory`                          — linear memory
//! - `abi_version() -> i32`            — must return `1`
//! - `alloc(len: i32) -> i32`          — reserve `len` bytes, return a pointer
//! - `transform(ptr: i32, len: i32) -> i64`
//!       — read the UTF-8 input at `[ptr, ptr+len)`, write UTF-8 output, and
//!         return `(out_ptr << 32) | out_len`.

use crate::config::AppConfig;
use crate::error::{EngineError, Result};
use crate::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Maximum fuel a single plugin call may burn. Generous for byte-shuffling, but
/// finite, so an unbounded loop terminates with an error instead of hanging.
const FUEL_BUDGET: u64 = 50_000_000;
/// Reject absurd output sizes (a plugin claiming a multi-GB result).
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
pub const ABI_VERSION: i32 = 1;

/// On-disk plugin manifest (`plugin.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// What the plugin does, e.g. `"metadata-transform"`. Informational in v1.
    #[serde(default)]
    pub kind: String,
    /// Wasm module filename, relative to the plugin directory.
    pub entry: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// Declared capabilities. v1 only honours pure-data transforms; anything
    /// else is recorded for display but grants no host access (none exists).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A discovered plugin plus its runtime state, for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub enabled: bool,
    /// True when the manifest parsed and the entry wasm file exists.
    pub valid: bool,
    /// Populated when discovery found a problem (missing wasm, bad manifest).
    pub error: Option<String>,
    pub dir: String,
}

impl Engine {
    pub fn plugins_dir(&self) -> PathBuf {
        self.paths.data_dir.join("plugins")
    }

    /// Discover every plugin under `<data_dir>/plugins`. Never fails on a single
    /// bad plugin: a broken manifest or missing wasm becomes an invalid entry
    /// with an `error`, so the UI can surface it without hiding the rest.
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        let enabled = self.load_config().enabled_plugins;
        let root = self.plugins_dir();
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&root) else {
            return out;
        };
        for entry in rd.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let dir_str = dir.to_string_lossy().to_string();
            match read_manifest(&dir) {
                Ok(manifest) => {
                    let wasm = dir.join(&manifest.entry);
                    let (valid, error) = if wasm.is_file() {
                        (true, None)
                    } else {
                        (false, Some(format!("entry wasm missing: {}", manifest.entry)))
                    };
                    let is_on = enabled.iter().any(|id| id == &manifest.id);
                    out.push(PluginInfo {
                        enabled: is_on,
                        valid,
                        error,
                        dir: dir_str,
                        manifest,
                    });
                }
                Err(e) => {
                    // Surface the broken plugin with a synthetic manifest so the
                    // user can see *something* is wrong in that directory.
                    let id = dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    out.push(PluginInfo {
                        manifest: PluginManifest {
                            id: id.clone(),
                            name: id,
                            version: "?".into(),
                            kind: String::new(),
                            entry: String::new(),
                            description: None,
                            author: None,
                            capabilities: Vec::new(),
                        },
                        enabled: false,
                        valid: false,
                        error: Some(e.to_string()),
                        dir: dir_str,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
        out
    }

    /// Enable or disable a plugin by id (persisted in config). Enabling an
    /// unknown id is rejected so the enabled-list stays meaningful.
    pub fn set_plugin_enabled(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        let mut cfg: AppConfig = self.load_config();
        if enabled {
            let known = self.list_plugins().into_iter().any(|p| p.manifest.id == plugin_id && p.valid);
            if !known {
                return Err(EngineError::NotFound(format!("valid plugin {plugin_id}")));
            }
            if !cfg.enabled_plugins.iter().any(|id| id == plugin_id) {
                cfg.enabled_plugins.push(plugin_id.to_string());
            }
        } else {
            cfg.enabled_plugins.retain(|id| id != plugin_id);
        }
        self.save_config(&cfg)
    }

    /// Run an enabled plugin's `transform` over `input`. Refuses disabled or
    /// invalid plugins. The wasm runs sandboxed (no host imports) and
    /// fuel-metered, so this is safe to call with untrusted plugin code.
    pub fn run_plugin_transform(&self, plugin_id: &str, input: &str) -> Result<String> {
        let plugin = self
            .list_plugins()
            .into_iter()
            .find(|p| p.manifest.id == plugin_id)
            .ok_or_else(|| EngineError::NotFound(format!("plugin {plugin_id}")))?;
        if !plugin.enabled {
            return Err(EngineError::Invalid(format!("plugin {plugin_id} is not enabled")));
        }
        if !plugin.valid {
            return Err(EngineError::Invalid(format!(
                "plugin {plugin_id} is invalid: {}",
                plugin.error.unwrap_or_default()
            )));
        }
        let wasm = std::fs::read(Path::new(&plugin.dir).join(&plugin.manifest.entry))?;
        run_wasm_transform(&wasm, input)
    }
}

fn read_manifest(dir: &Path) -> Result<PluginManifest> {
    let path = dir.join("plugin.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| EngineError::Invalid(format!("plugin.json unreadable: {e}")))?;
    let manifest: PluginManifest = serde_json::from_str(&text)
        .map_err(|e| EngineError::Invalid(format!("plugin.json invalid: {e}")))?;
    if manifest.id.trim().is_empty() || manifest.entry.trim().is_empty() {
        return Err(EngineError::Invalid("plugin.json missing id or entry".into()));
    }
    Ok(manifest)
}

/// Execute a data-only wasm transform in a fully sandboxed interpreter: no host
/// functions are linked, and the call is fuel-metered. Pure (takes bytes), so it
/// is unit-tested without any plugin on disk.
fn run_wasm_transform(wasm: &[u8], input: &str) -> Result<String> {
    use wasmi::{Config, Engine as WasmEngine, Linker, Module, Store};

    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = WasmEngine::new(&config);
    let module = Module::new(&engine, wasm)
        .map_err(|e| EngineError::Invalid(format!("plugin wasm invalid: {e}")))?;
    let mut store = Store::new(&engine, ());
    store
        .add_fuel(FUEL_BUDGET)
        .map_err(|e| EngineError::Invalid(format!("fuel init: {e}")))?;

    // No imports are defined on the linker: the plugin runs with zero host
    // access. Any import it declares fails instantiation here.
    let linker = Linker::<()>::new(&engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| EngineError::Invalid(format!("plugin instantiation failed (imports are not permitted): {e}")))?
        .start(&mut store)
        .map_err(|e| EngineError::Invalid(format!("plugin start failed: {e}")))?;

    let abi = instance
        .get_typed_func::<(), i32>(&store, "abi_version")
        .map_err(|_| EngineError::Invalid("plugin missing abi_version export".into()))?;
    let version = abi
        .call(&mut store, ())
        .map_err(|e| EngineError::Invalid(format!("abi_version trapped: {e}")))?;
    if version != ABI_VERSION {
        return Err(EngineError::Invalid(format!(
            "plugin ABI {version} unsupported (engine speaks {ABI_VERSION})"
        )));
    }

    let alloc = instance
        .get_typed_func::<i32, i32>(&store, "alloc")
        .map_err(|_| EngineError::Invalid("plugin missing alloc export".into()))?;
    let transform = instance
        .get_typed_func::<(i32, i32), i64>(&store, "transform")
        .map_err(|_| EngineError::Invalid("plugin missing transform export".into()))?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| EngineError::Invalid("plugin missing memory export".into()))?;

    let bytes = input.as_bytes();
    let len = i32::try_from(bytes.len())
        .map_err(|_| EngineError::Invalid("plugin input too large".into()))?;
    let ptr = alloc
        .call(&mut store, len)
        .map_err(|e| EngineError::Invalid(format!("alloc trapped: {e}")))?;
    memory
        .write(&mut store, ptr as usize, bytes)
        .map_err(|e| EngineError::Invalid(format!("input write failed: {e}")))?;

    let packed = transform
        .call(&mut store, (ptr, len))
        .map_err(|e| EngineError::Invalid(format!("transform trapped (or out of fuel): {e}")))?;
    let out_ptr = (packed >> 32) as u32 as usize;
    let out_len = (packed & 0xffff_ffff) as u32 as usize;
    if out_len > MAX_OUTPUT {
        return Err(EngineError::Invalid("plugin output exceeds limit".into()));
    }
    if out_ptr.saturating_add(out_len) > memory.data(&store).len() {
        return Err(EngineError::Invalid("plugin output out of bounds".into()));
    }
    let mut buf = vec![0u8; out_len];
    memory
        .read(&store, out_ptr, &mut buf)
        .map_err(|e| EngineError::Invalid(format!("output read failed: {e}")))?;
    String::from_utf8(buf).map_err(|_| EngineError::Invalid("plugin output not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal, valid data-only plugin: uppercases ASCII via the v1 ABI.
    const UPPERCASE_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (global $heap (mut i32) (i32.const 1024))
      (func $alloc (export "alloc") (param $n i32) (result i32)
        (local $p i32)
        (local.set $p (global.get $heap))
        (global.set $heap (i32.add (global.get $heap) (local.get $n)))
        (local.get $p))
      (func (export "abi_version") (result i32) (i32.const 1))
      (func (export "transform") (param $ptr i32) (param $len i32) (result i64)
        (local $out i32)
        (local $i i32)
        (local $b i32)
        (local.set $out (call $alloc (local.get $len)))
        (local.set $i (i32.const 0))
        (block $done
          (loop $loop
            (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
            (local.set $b (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
            (if (i32.and (i32.ge_u (local.get $b) (i32.const 97))
                         (i32.le_u (local.get $b) (i32.const 122)))
              (then (local.set $b (i32.sub (local.get $b) (i32.const 32)))))
            (i32.store8 (i32.add (local.get $out) (local.get $i)) (local.get $b))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $loop)))
        (i64.or
          (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
          (i64.extend_i32_u (local.get $len)))))
    "#;

    /// A hostile plugin that loops forever — must be killed by fuel metering.
    const SPIN_WAT: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "abi_version") (result i32) (i32.const 1))
      (func (export "alloc") (param $n i32) (result i32) (i32.const 1024))
      (func (export "transform") (param $ptr i32) (param $len i32) (result i64)
        (loop $l (br $l))
        (i64.const 0)))
    "#;

    /// A plugin that tries to import a host function — must be refused.
    const IMPORT_WAT: &str = r#"
    (module
      (import "env" "evil" (func $evil))
      (memory (export "memory") 1)
      (func (export "abi_version") (result i32) (i32.const 1))
      (func (export "alloc") (param $n i32) (result i32) (i32.const 1024))
      (func (export "transform") (param $ptr i32) (param $len i32) (result i64)
        (call $evil)
        (i64.const 0)))
    "#;

    #[test]
    fn runs_data_only_transform() {
        let wasm = wat::parse_str(UPPERCASE_WAT).unwrap();
        let out = run_wasm_transform(&wasm, "hello world").unwrap();
        assert_eq!(out, "HELLO WORLD");
    }

    #[test]
    fn infinite_loop_is_stopped_by_fuel() {
        let wasm = wat::parse_str(SPIN_WAT).unwrap();
        let err = run_wasm_transform(&wasm, "x").unwrap_err();
        assert!(matches!(err, EngineError::Invalid(_)), "{err:?}");
    }

    #[test]
    fn host_imports_are_refused() {
        let wasm = wat::parse_str(IMPORT_WAT).unwrap();
        let err = run_wasm_transform(&wasm, "x").unwrap_err();
        // Instantiation must fail because the linker defines no imports.
        assert!(matches!(err, EngineError::Invalid(_)));
    }
}
