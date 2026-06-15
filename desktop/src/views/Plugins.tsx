import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import { PageHeader } from "../components/PageHeader";
import type { PluginInfo } from "../api/types";

export function Plugins() {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);

  const plugins = useQuery({
    queryKey: ["plugins"],
    queryFn: () => api.listPlugins(),
  });
  const dir = useQuery({ queryKey: ["plugins-dir"], queryFn: () => api.pluginsDir() });

  const toggle = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api.setPluginEnabled(id, enabled),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["plugins"] }),
    onError: (e: unknown) => setToast(`Couldn't update plugin: ${String(e)}`),
  });

  const list = plugins.data ?? [];

  return (
    <div className="animate-fade-up max-w-3xl">
      <PageHeader
        title="Plugins"
        subtitle="Community WebAssembly modules, sandboxed."
        aside={
          list.length > 0 ? (
            <span className="glass rounded-full px-3.5 py-1.5 font-display text-sm font-bold text-primary">
              {list.length}
            </span>
          ) : undefined
        }
      />

      <div className="glass relative mb-5 overflow-hidden rounded-2xl p-4">
        <span className="absolute inset-y-0 left-0 w-1 bg-gradient-to-b from-primary to-secondary" />
        <p className="max-w-2xl pl-2 text-xs leading-relaxed text-ink-dim">
          Plugins run in a sandbox with no access to your files, network, or
          system — they can only transform text passed to them, and are halted if
          they run too long. Drop a plugin folder (manifest +{" "}
          <span className="font-mono text-ink">.wasm</span>) into the directory
          below, then reload.
        </p>
      </div>
      {dir.data && (
        <div className="glass mb-5 flex items-center gap-3 rounded-xl p-3">
          <span className="truncate font-mono text-xs text-ink-dim">{dir.data}</span>
          <Focusable
            onActivate={() => {
              navigator.clipboard?.writeText(dir.data!);
              setToast("Plugin folder path copied.");
            }}
            ariaLabel="Copy plugins folder path"
            className="ml-auto shrink-0 rounded-lg px-2 py-1 text-xs text-primary"
          >
            Copy path
          </Focusable>
          <Focusable
            onActivate={() => qc.invalidateQueries({ queryKey: ["plugins"] })}
            ariaLabel="Reload plugins"
            className="shrink-0 rounded-lg px-2 py-1 text-xs"
          >
            Reload
          </Focusable>
        </div>
      )}

      {list.length === 0 ? (
        <p className="text-sm text-ink-dim">No plugins installed.</p>
      ) : (
        <ul className="flex flex-col gap-3">
          {list.map((p) => (
            <PluginRow
              key={p.manifest.id}
              p={p}
              onToggle={(enabled) => toggle.mutate({ id: p.manifest.id, enabled })}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function PluginRow({
  p,
  onToggle,
}: {
  p: PluginInfo;
  onToggle: (enabled: boolean) => void;
}) {
  const [testOpen, setTestOpen] = useState(false);
  return (
    <li
      className={`glass relative overflow-hidden rounded-2xl p-4 ${
        p.enabled && p.valid ? "ring-1 ring-primary/30" : ""
      }`}
    >
      {p.enabled && p.valid && (
        <span className="absolute inset-y-0 left-0 w-1 bg-gradient-to-b from-primary to-secondary" />
      )}
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold">{p.manifest.name}</span>
            <span className="text-[11px] text-ink-dim">v{p.manifest.version}</span>
            <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-primary/80 ring-1 ring-primary/15">
              {p.manifest.kind}
            </span>
            {!p.valid && (
              <span className="rounded-full bg-secondary/20 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-secondary">
                invalid
              </span>
            )}
          </div>
          {p.manifest.description && (
            <p className="mt-1 text-xs text-ink-dim">{p.manifest.description}</p>
          )}
          {p.manifest.author && (
            <p className="mt-0.5 text-[11px] text-ink-dim/70">by {p.manifest.author}</p>
          )}
          {p.error && (
            <p className="mt-1 font-mono text-[11px] text-secondary">{p.error}</p>
          )}
        </div>
        <Focusable
          onActivate={() => p.valid && onToggle(!p.enabled)}
          ariaLabel={p.enabled ? "Disable plugin" : "Enable plugin"}
          className={`shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold ${
            !p.valid
              ? "bg-surface-2 text-ink-dim/50"
              : p.enabled
                ? "bg-primary text-black"
                : "glass text-ink"
          }`}
        >
          {p.enabled ? "Enabled" : "Enable"}
        </Focusable>
      </div>

      {p.valid && p.manifest.kind === "transform" && (
        <div className="mt-3 border-t border-primary/10 pt-3">
          <Focusable
            onActivate={() => setTestOpen((o) => !o)}
            ariaLabel="Toggle plugin tester"
            className="text-xs text-primary"
          >
            {testOpen ? "Hide tester" : "Test transform…"}
          </Focusable>
          {testOpen && <Tester pluginId={p.manifest.id} />}
        </div>
      )}
    </li>
  );
}

function Tester({ pluginId }: { pluginId: string }) {
  const [input, setInput] = useState("");
  const [output, setOutput] = useState<string | null>(null);
  const setToast = useStore((s) => s.setToast);

  const run = useMutation({
    mutationFn: () => api.runPluginTransform(pluginId, input),
    onSuccess: (out) => setOutput(out),
    onError: (e: unknown) => setToast(`Plugin error: ${String(e)}`),
  });

  return (
    <div className="mt-3 flex flex-col gap-2">
      <textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="Input text…"
        rows={3}
        className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
      />
      <Focusable
        onActivate={() => run.mutate()}
        ariaLabel="Run transform"
        className="self-start rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
      >
        {run.isPending ? "Running…" : "Run"}
      </Focusable>
      {output !== null && (
        <pre className="glass overflow-x-auto rounded-lg bg-surface-2 p-3 text-xs">
          {output}
        </pre>
      )}
    </div>
  );
}
