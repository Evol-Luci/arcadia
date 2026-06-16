//! Minimal ZIP reader — lists entry filenames from the central directory.
//!
//! RetroArch (and most libretro cores) load zipped content natively, so a
//! `Super Mario World.zip` holding one `.sfc` is a SNES game, while an arcade
//! romset is a `.zip` of many small files identified by its set name. To tell
//! these apart we only need the *names* of the entries, never their contents —
//! so this reader walks the End-of-Central-Directory record and the central
//! directory, and decompresses nothing.
//!
//! Deliberately tiny: no ZIP64, no decompression, no encryption handling. Any
//! unexpected structure returns `None` and the caller falls back to a default.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// End-of-Central-Directory record signature (`PK\x05\x06`).
const EOCD_SIGNATURE: u32 = 0x0605_4b50;
/// Central-directory file-header signature (`PK\x01\x02`).
const CDFH_SIGNATURE: u32 = 0x0201_4b50;
/// EOCD is 22 bytes plus an optional comment of up to 0xFFFF bytes.
const EOCD_MAX: usize = 22 + 0xFFFF;
/// Sentinel meaning "see the ZIP64 record", which we don't parse.
const ZIP64_SENTINEL: u32 = 0xFFFF_FFFF;

/// List the entry filenames stored in a ZIP archive. `None` if the file isn't a
/// readable ZIP or uses ZIP64.
pub fn entry_names(path: &Path) -> Option<Vec<String>> {
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();

    // Read the tail and locate the EOCD by scanning backwards for its signature.
    let tail_len = EOCD_MAX.min(size as usize);
    let tail = read_at(&mut f, size - tail_len as u64, tail_len)?;
    let eocd = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| le_u32(&tail, i) == Some(EOCD_SIGNATURE))?;

    let cd_size = le_u32(&tail, eocd + 12)? as u64;
    let cd_offset = le_u32(&tail, eocd + 16)?;
    if cd_offset == ZIP64_SENTINEL || cd_size as u32 == ZIP64_SENTINEL {
        return None; // ZIP64 — out of scope for this tiny reader.
    }

    let cd = read_at(&mut f, cd_offset as u64, cd_size as usize)?;
    Some(parse_central_directory(&cd))
}

/// List the file entry names in a `.7z` archive, reading only its header (the
/// file table) and decompressing none of the payload. `None` if the file isn't a
/// readable 7z. Directory entries are dropped so the caller sees the same shape
/// it gets from [`entry_names`] (file names only).
///
/// A 7z header is itself often LZMA-compressed, so unlike the ZIP reader this
/// decodes the (small) header stream — but never a content stream. This is what
/// lets a 7z-compressed disc image (`Mario Paint.sfc`, `Game [ULUS…].iso`, …) be
/// routed by its inner filename instead of defaulting to an arcade romset.
pub fn entry_names_7z(path: &Path) -> Option<Vec<String>> {
    let archive = sevenz_rust2::Archive::open(path).ok()?;
    Some(
        archive
            .files
            .iter()
            .filter(|e| !e.is_directory())
            .map(|e| e.name().to_string())
            .collect(),
    )
}

/// Walk central-directory file headers, collecting each entry's name.
fn parse_central_directory(cd: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut pos = 0usize;
    while pos + 46 <= cd.len() {
        if le_u32(cd, pos) != Some(CDFH_SIGNATURE) {
            break;
        }
        let name_len = le_u16(cd, pos + 28).unwrap_or(0) as usize;
        let extra_len = le_u16(cd, pos + 30).unwrap_or(0) as usize;
        let comment_len = le_u16(cd, pos + 32).unwrap_or(0) as usize;
        let name_start = pos + 46;
        let name_end = name_start + name_len;
        if name_end > cd.len() {
            break;
        }
        names.push(String::from_utf8_lossy(&cd[name_start..name_end]).into_owned());
        pos = name_end + extra_len + comment_len;
    }
    names
}

fn read_at(f: &mut std::fs::File, offset: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = Vec::new();
    f.take(len as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A zipped game extracted to a directory so an emulator that can't read
/// archives natively can boot it. The directory is *stable per archive* and
/// persists across launches (see [`extract_for_launch`]): an emulator savestate
/// embeds the absolute path of the disc image it was made from, so that path has
/// to resolve the next time the game is launched. A random per-launch temp dir
/// (the old behaviour) broke launch-into-state for archived discs — DuckStation
/// would report "Failed to open CD image from save state".
pub struct ExtractedRom {
    /// The entry the emulator should boot (an absolute path inside the extraction
    /// dir). Stable across launches for a given archive.
    pub path: PathBuf,
}

/// Root under which per-archive extraction dirs live. Honours `XDG_CACHE_HOME`
/// (so it's overridable, and tests can point it at a temp dir), else
/// `~/.cache/arcadia`, else the system temp dir if `$HOME` is unset too.
///
/// Why not the system `/tmp`: most of our emulators are Flatpaks, and a Flatpak
/// sandbox gets a *private* `/tmp` — it can't see the host's `/tmp/arcadia-rom-*`,
/// so the emulator fails with "Failed to open … for reading". Every Flatpak we
/// ship grants `home`/`host` filesystem access, so an absolute path under
/// `$HOME` is readable from inside the sandbox. `~/.cache` is the natural spot
/// for regenerable files.
fn extract_base_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("arcadia");
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".cache/arcadia"),
        None => std::env::temp_dir().join("arcadia"),
    }
}

/// A stable directory name for an archive's extraction, `arcadia-rom-<hash>`,
/// where the hash is an FNV-1a digest of the archive's canonical absolute path.
/// Deterministic across launches and app restarts so the extracted disc keeps
/// the same absolute path — what makes an emulator's path-embedding savestate
/// reload correctly.
fn stable_dir_name(archive_path: &Path) -> String {
    let abs = std::fs::canonicalize(archive_path).unwrap_or_else(|_| archive_path.to_path_buf());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for b in abs.as_os_str().as_encoded_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    }
    format!("arcadia-rom-{hash:016x}")
}

/// If a previous extraction of this archive is still on disk, return the entry
/// the emulator should boot — so we reuse it instead of re-extracting (and,
/// crucially, keep the same absolute path a savestate may reference).
fn existing_launch_target(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    for entry in std::fs::read_dir(dir).ok()? {
        let entry = entry.ok()?;
        let meta = entry.metadata().ok()?;
        if meta.is_file() {
            files.push((entry.path(), meta.len()));
        }
    }
    pick_launch_target(&files)
}

/// The already-extracted launch target for an archive, if a prior launch left one
/// on disk — *without* extracting. Savestate discovery uses this: an emulator that
/// keys states by the ROM (beside it, or by its stem) writes them next to the
/// *extracted* file in the cache and under the extracted file's *inner* name,
/// which can differ from the archive's name (e.g. a `.zip` named "Kirby (U)" whose
/// inner ROM is "0028 - Kirby - Canvas Curse"). Returns `None` for a ROM that was
/// never extracted (no archive, or never launched), so callers fall back to the
/// original path.
pub(crate) fn existing_extraction(archive_path: &Path) -> Option<PathBuf> {
    let dir = extract_base_dir().join(stable_dir_name(archive_path));
    existing_launch_target(&dir)
}

/// Extract a compressed game to a fresh temp dir and pick the entry to boot.
///
/// Dispatches on the archive format: `.7z` goes through the 7z decoder, anything
/// else is treated as a ZIP. Both write every file entry out — flattened to its
/// base name so that multi-file formats survive (a `.cue` and its `.bin` land
/// side by side and the sheet's relative reference still resolves) and so that
/// crafted paths can't escape the temp dir (classic "zip-slip").
///
/// Returns `None` if the file isn't a readable archive of that format or holds no
/// file entries; the caller then falls back to handing the emulator the original
/// archive.
pub fn extract_for_launch(archive_path: &Path) -> Option<ExtractedRom> {
    extract_for_launch_in(archive_path, &extract_base_dir())
}

/// [`extract_for_launch`] against an explicit base root. The extraction dir is
/// `<base>/arcadia-rom-<hash>`, stable per archive; if it already holds a
/// bootable entry from a prior launch we reuse it (no re-extraction), keeping the
/// disc image's absolute path identical so savestates that embed it still load.
pub(crate) fn extract_for_launch_in(archive_path: &Path, base: &Path) -> Option<ExtractedRom> {
    let dir = base.join(stable_dir_name(archive_path));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "extract: can't create extraction dir");
        return None;
    }
    if let Some(path) = existing_launch_target(&dir) {
        return Some(ExtractedRom { path });
    }
    if has_ext(archive_path, "7z") {
        extract_7z_for_launch(archive_path, &dir)
    } else {
        extract_zip_for_launch(archive_path, &dir)
    }
}

/// Extract a `.zip` game. See [`extract_for_launch`] for the shared contract.
///
/// Decompression is the only place we pull in the `zip` crate; everywhere else
/// this module stays a tiny reader.
fn extract_zip_for_launch(zip_path: &Path, dir: &Path) -> Option<ExtractedRom> {
    // Failures here are silently retried as "hand the emulator the raw archive",
    // which then can't boot — indistinguishable from "the game won't load". So we
    // log each failure point with its cause; the most common real-world one is
    // running out of space in the temp filesystem while writing a large image
    // (e.g. a multi-GB PS2 ISO into a too-small `/tmp` tmpfs).
    let file = match std::fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(archive = %zip_path.display(), error = %e, "extract: can't open archive");
            return None;
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(archive = %zip_path.display(), error = %e, "extract: not a readable zip");
            return None;
        }
    };

    let mut extracted: Vec<(PathBuf, u64)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(index = i, error = %e, "extract: can't read zip entry");
                return None;
            }
        };
        if !entry.is_file() {
            continue;
        }
        // Flatten to the final path component: this drops any directory prefix
        // (and with it any `..` traversal or absolute path), so every entry
        // lands directly in our temp dir and can't escape it.
        let base = match Path::new(entry.name()).file_name() {
            Some(b) => PathBuf::from(b),
            None => continue,
        };
        let out_path = dir.join(&base);
        let mut out = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(path = %out_path.display(), error = %e, "extract: can't create output file");
                return None;
            }
        };
        // The likely failure for big images: temp filesystem full mid-write.
        if let Err(e) = std::io::copy(&mut entry, &mut out) {
            tracing::warn!(
                entry = %entry.name(), bytes = entry.size(), error = %e,
                "extract: failed writing entry (temp filesystem full?)"
            );
            return None;
        }
        extracted.push((out_path, entry.size()));
    }

    let path = pick_launch_target(&extracted)?;
    Some(ExtractedRom { path })
}

/// Extract a `.7z` game. See [`extract_for_launch`] for the shared contract.
///
/// 7z is the common PS2/PSP distribution format, so this is what rescues a
/// `.7z`-compressed disc image for an emulator (PCSX2, …) that can only boot a
/// raw ISO/CUE. We let `sevenz_rust2` decode each entry and write it out with the
/// same flatten-and-pick rules as the ZIP path.
fn extract_7z_for_launch(sevenz_path: &Path, dir: &Path) -> Option<ExtractedRom> {
    let mut archive = match sevenz_rust2::ArchiveReader::open(
        sevenz_path,
        sevenz_rust2::Password::empty(),
    ) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(archive = %sevenz_path.display(), error = %e, "extract: not a readable 7z");
            return None;
        }
    };

    let dir_path = dir.to_path_buf();
    let mut extracted: Vec<(PathBuf, u64)> = Vec::new();
    let mut write_error: Option<String> = None;
    // `for_each_entries` streams each entry's decompressed bytes; we flatten the
    // name to its final component (drops directory prefixes and any `..`
    // traversal) and write it directly into our temp dir.
    let result = archive.for_each_entries(|entry, reader| {
        if entry.is_directory() {
            return Ok(true);
        }
        let base = match Path::new(entry.name()).file_name() {
            Some(b) => PathBuf::from(b),
            None => return Ok(true),
        };
        let out_path = dir_path.join(&base);
        let mut out = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(e) => {
                write_error = Some(format!("can't create {}: {e}", out_path.display()));
                return Ok(false);
            }
        };
        // The likely failure for big images: temp filesystem full mid-write.
        match std::io::copy(reader, &mut out) {
            Ok(n) => {
                extracted.push((out_path, n));
                Ok(true)
            }
            Err(e) => {
                write_error = Some(format!(
                    "failed writing entry {} (temp filesystem full?): {e}",
                    entry.name()
                ));
                Ok(false)
            }
        }
    });
    if let Err(e) = result {
        tracing::warn!(archive = %sevenz_path.display(), error = %e, "extract: 7z decode failed");
        return None;
    }
    if let Some(msg) = write_error {
        tracing::warn!(archive = %sevenz_path.display(), error = %msg, "extract: 7z write failed");
        return None;
    }

    let path = pick_launch_target(&extracted)?;
    Some(ExtractedRom { path })
}

/// Choose which extracted file to hand the emulator. A disc sheet/playlist
/// describes the others, so it wins when present (m3u points at cues, cues at
/// bins). Otherwise the largest file is the actual ROM — config or readme files
/// shipped alongside are always smaller.
fn pick_launch_target(files: &[(PathBuf, u64)]) -> Option<PathBuf> {
    const SHEETS: &[&str] = &["m3u", "cue", "gdi"];
    for ext in SHEETS {
        if let Some((p, _)) = files.iter().find(|(p, _)| has_ext(p, ext)) {
            return Some(p.clone());
        }
    }
    files.iter().max_by_key(|(_, size)| *size).map(|(p, _)| p.clone())
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

#[cfg(test)]
pub(crate) mod testfixtures {
    //! Build a real (stored, uncompressed) ZIP with given entry names so the
    //! reader can be exercised without shelling out to a zip tool.

    /// A minimal valid ZIP containing each name as a stored, empty entry.
    pub fn zip_with_names(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut offsets = Vec::new();

        for name in names {
            offsets.push(out.len() as u32);
            // Local file header (30 fixed bytes, then the name).
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // signature
            out.extend_from_slice(&[0u8; 4]); // version needed + flags
            out.extend_from_slice(&0u16.to_le_bytes()); // method: store
            out.extend_from_slice(&[0u8; 4]); // mod time + date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc
            out.extend_from_slice(&0u32.to_le_bytes()); // compressed size
            out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
            out.extend_from_slice(&(name.len() as u16).to_le_bytes()); // name len
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
        }

        for (name, &off) in names.iter().zip(&offsets) {
            // Central-directory file header (46 fixed bytes, then the name).
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // signature
            central.extend_from_slice(&[0u8; 6]); // version made/needed + flags
            central.extend_from_slice(&0u16.to_le_bytes()); // method
            central.extend_from_slice(&[0u8; 4]); // mod time + date
            central.extend_from_slice(&0u32.to_le_bytes()); // crc
            central.extend_from_slice(&0u32.to_le_bytes()); // compressed size
            central.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
            central.extend_from_slice(&(name.len() as u16).to_le_bytes()); // name len
            central.extend_from_slice(&0u16.to_le_bytes()); // extra len
            central.extend_from_slice(&0u16.to_le_bytes()); // comment len
            central.extend_from_slice(&[0u8; 8]); // disk start + internal/external attrs
            central.extend_from_slice(&off.to_le_bytes()); // local header offset
            central.extend_from_slice(name.as_bytes());
        }

        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);

        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // this disk
        out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        out.extend_from_slice(&(names.len() as u16).to_le_bytes()); // entries (disk)
        out.extend_from_slice(&(names.len() as u16).to_le_bytes()); // entries (total)
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn lists_single_entry() {
        let f = write_temp(&testfixtures::zip_with_names(&["Super Mario World.sfc"]));
        assert_eq!(entry_names(f.path()).unwrap(), vec!["Super Mario World.sfc"]);
    }

    #[test]
    fn lists_many_entries() {
        let names = ["maincpu.u1", "gfx1.u2", "sound.u3"];
        let f = write_temp(&testfixtures::zip_with_names(&names));
        assert_eq!(entry_names(f.path()).unwrap(), names);
    }

    #[test]
    fn non_zip_returns_none() {
        let f = write_temp(b"definitely not a zip file");
        assert!(entry_names(f.path()).is_none());
    }

    #[test]
    fn sevenz_lists_entry_names() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let tmp = tempfile::Builder::new().suffix(".7z").tempfile().unwrap();
        let mut w = ArchiveWriter::create(tmp.path()).unwrap();
        w.set_encrypt_header(false);
        for name in ["Daxter [ULUS10041].iso", "readme.txt"] {
            w.push_archive_entry(ArchiveEntry::new_file(name), Some(&[0u8; 32][..]))
                .unwrap();
        }
        w.finish().unwrap();
        let names = entry_names_7z(tmp.path()).unwrap();
        assert_eq!(names, vec!["Daxter [ULUS10041].iso", "readme.txt"]);
    }

    #[test]
    fn sevenz_non_archive_returns_none() {
        let f = write_temp(b"not a 7z either");
        assert!(entry_names_7z(f.path()).is_none());
    }

    /// Write a real ZIP with the given (name, bytes, compressed?) entries.
    fn build_zip(entries: &[(&str, Vec<u8>, bool)]) -> tempfile::NamedTempFile {
        use zip::write::SimpleFileOptions;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut zw = zip::ZipWriter::new(std::fs::File::create(tmp.path()).unwrap());
        for (name, bytes, deflate) in entries {
            let method = if *deflate {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            };
            zw.start_file(*name, SimpleFileOptions::default().compression_method(method))
                .unwrap();
            zw.write_all(bytes).unwrap();
        }
        zw.finish().unwrap();
        tmp
    }

    #[test]
    fn extracts_deflated_single_rom() {
        let base = tempfile::tempdir().unwrap();
        // Highly compressible payload so DEFLATE actually engages.
        let payload = vec![0x42u8; 8000];
        let f = build_zip(&[("Sonic the Hedgehog.md", payload.clone(), true)]);
        let ex = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        assert!(ex.path.to_string_lossy().ends_with("Sonic the Hedgehog.md"));
        assert_eq!(std::fs::read(&ex.path).unwrap(), payload);
    }

    #[test]
    fn picks_cue_sheet_and_keeps_bin_alongside() {
        let base = tempfile::tempdir().unwrap();
        // The .bin is larger, but the .cue is what the emulator must boot.
        let f = build_zip(&[
            ("game.bin", vec![0u8; 9000], false),
            ("game.cue", b"FILE \"game.bin\" BINARY".to_vec(), false),
        ]);
        let ex = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        assert!(ex.path.to_string_lossy().ends_with("game.cue"));
        assert!(ex.path.parent().unwrap().join("game.bin").exists());
    }

    #[test]
    fn extraction_persists_and_reuses_same_path() {
        // The disc path must be stable across launches so an emulator savestate
        // that embeds it still resolves — i.e. the dir survives drop and a second
        // call returns the identical path without re-extracting.
        let base = tempfile::tempdir().unwrap();
        let f = build_zip(&[("rom.gba", vec![1u8; 1000], true)]);
        let first = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        let path = first.path.clone();
        drop(first);
        assert!(path.exists(), "extraction must persist after the guard drops");
        let second = extract_for_launch_in(f.path(), base.path()).expect("reused");
        assert_eq!(second.path, path, "second launch must reuse the same path");
    }

    #[test]
    fn nested_paths_are_flattened_not_escaped() {
        let base = tempfile::tempdir().unwrap();
        // A crafted traversal name must not write outside the extraction dir.
        let f = build_zip(&[("../../etc/evil.sfc", vec![7u8; 500], false)]);
        let ex = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        assert_eq!(ex.path.file_name().unwrap(), "evil.sfc");
        // The traversal prefix was stripped — no `etc` component survives.
        assert!(!ex.path.components().any(|c| c.as_os_str() == "etc"));
    }

    /// Write a real `.7z` holding each (name, bytes) entry, LZMA2-compressed —
    /// the format 7-Zip produces by default and the one game archives use.
    fn build_7z(entries: &[(&str, Vec<u8>)]) -> tempfile::NamedTempFile {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let tmp = tempfile::Builder::new().suffix(".7z").tempfile().unwrap();
        let mut w = ArchiveWriter::create(tmp.path()).unwrap();
        w.set_encrypt_header(false);
        for (name, bytes) in entries {
            w.push_archive_entry(ArchiveEntry::new_file(*name), Some(&bytes[..]))
                .unwrap();
        }
        w.finish().unwrap();
        tmp
    }

    #[test]
    fn extracts_compressed_7z_single_rom() {
        let base = tempfile::tempdir().unwrap();
        // Highly compressible payload so the codec actually engages.
        let payload = vec![0x42u8; 8000];
        let f = build_7z(&[("Jak and Daxter [SCUS-97124].iso", payload.clone())]);
        let ex = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        assert!(ex.path.to_string_lossy().ends_with("Jak and Daxter [SCUS-97124].iso"));
        assert_eq!(std::fs::read(&ex.path).unwrap(), payload);
    }

    #[test]
    fn sevenz_picks_cue_sheet_and_keeps_bin_alongside() {
        let base = tempfile::tempdir().unwrap();
        let f = build_7z(&[
            ("game.bin", vec![0u8; 9000]),
            ("game.cue", b"FILE \"game.bin\" BINARY".to_vec()),
        ]);
        let ex = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        assert!(ex.path.to_string_lossy().ends_with("game.cue"));
        assert!(ex.path.parent().unwrap().join("game.bin").exists());
    }

    #[test]
    fn sevenz_extraction_persists_and_reuses_same_path() {
        let base = tempfile::tempdir().unwrap();
        let f = build_7z(&[("rom.iso", vec![1u8; 4000])]);
        let first = extract_for_launch_in(f.path(), base.path()).expect("extracted");
        let path = first.path.clone();
        drop(first);
        assert!(path.exists(), "extraction must persist after the guard drops");
        let second = extract_for_launch_in(f.path(), base.path()).expect("reused");
        assert_eq!(second.path, path, "second launch must reuse the same path");
    }

    #[test]
    fn sevenz_non_archive_returns_none_for_launch() {
        let base = tempfile::tempdir().unwrap();
        let f = write_temp(b"definitely not a 7z");
        // A `.7z` suffix routes to the 7z path; a corrupt file must not panic.
        let named = tempfile::Builder::new().suffix(".7z").tempfile().unwrap();
        std::fs::copy(f.path(), named.path()).unwrap();
        assert!(extract_for_launch_in(named.path(), base.path()).is_none());
    }
}
