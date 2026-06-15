//! Minimal disc-image reader — just enough to route ambiguous `.iso` files.
//!
//! `.iso` is shared by PS2, PSP, and Nintendo's GameCube/Wii, and filenames
//! frequently carry no title ID (`Ace Combat X.iso`), so a name-only heuristic
//! mislabels the very files we care about. Instead we look at the image's
//! structure, mirroring the detection already used for PS3 folder dumps:
//!
//!   * a **GameCube** or **Wii** image is a *raw* disc dump — not ISO 9660 — that
//!     carries a magic word in its header (`0xC2339F3D` at 0x1C for GameCube,
//!     `0x5D1C9EA3` at 0x18 for Wii) and an internal game name at 0x20;
//!   * a **PSP UMD** is ISO 9660 with a `PSP_GAME` directory at the root, whose
//!     `PSP_GAME/PARAM.SFO` carries the real `TITLE`;
//!   * a **PS2 disc** is ISO 9660 with `SYSTEM.CNF` at the root (it points at the
//!     `SLUS_…` boot ELF).
//!
//! We read only the handful of sectors needed to identify the image, never the
//! whole thing. The ISO 9660 path is a deliberately tiny reader: no Joliet, no
//! Rock Ridge, no path tables — just the primary volume descriptor and the
//! records we need. Anything unexpected returns `None` and the caller falls back
//! to the filename heuristic.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// ISO 9660 logical sector size.
const SECTOR: u64 = 2048;
/// The primary volume descriptor always lives at logical sector 16.
const PVD_LBA: u64 = 16;
/// Offset of the root directory record within the PVD.
const PVD_ROOT_RECORD: usize = 156;
/// Directory-record flag bit marking an entry as a subdirectory.
const FLAG_DIRECTORY: u8 = 0x02;

/// GameCube disc magic, big-endian, at header offset 0x1C.
const GAMECUBE_MAGIC: u32 = 0xC233_9F3D;
/// Wii disc magic, big-endian, at header offset 0x18.
const WII_MAGIC: u32 = 0x5D1C_9EA3;
/// Byte offset of the null-terminated internal game name in the disc header.
const DISC_NAME_OFFSET: u64 = 0x20;
/// Maximum bytes the internal game name occupies in the header.
const DISC_NAME_LEN: usize = 64;

/// The 12-byte sync pattern that opens every raw (2352-byte) CD sector:
/// `00 FF×10 00`. Cooked (`.iso`, 2048-byte) images don't have it; we sniff for
/// it to decide a track's sector layout.
const RAW_SYNC: [u8; 12] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
];

/// XDVDFS volume signature stamped at sector 32 of an Xbox game partition.
const XBOX_SIGNATURE: &[u8] = b"MICROSOFT*XBOX*MEDIA";
/// Absolute byte offsets where [`XBOX_SIGNATURE`] can sit: sector 32 (0x10000)
/// past each known partition base — trimmed/extracted XISO, then XGD3, XGD1, and
/// the XGD2 "global" redump layout. The first that matches wins.
const XBOX_SIGNATURE_OFFSETS: &[u64] = &[0x1_0000, 0x209_0000, 0x1831_0000, 0xFDA_0000];

/// What a readable disc image turned out to be.
pub enum Disc {
    /// A PSP UMD image. `title` is the `PARAM.SFO` TITLE when readable.
    Psp { title: Option<String> },
    /// A PlayStation 2 disc image.
    Ps2,
    /// A Nintendo GameCube disc image. `title` is the internal game name.
    GameCube { title: Option<String> },
    /// A Nintendo Wii disc image. `title` is the internal game name.
    Wii { title: Option<String> },
    /// An original Xbox disc image (XDVDFS).
    Xbox,
}

/// One ISO 9660 directory record we care about: where its data lives and
/// whether it's a subdirectory.
#[derive(Clone)]
struct Entry {
    lba: u32,
    len: u32,
    is_dir: bool,
}

/// How a track stores its sectors. CD images come either *cooked* — just the
/// 2048-byte user area, as in a `.iso` — or *raw*, the full 2352-byte CD sector
/// (12-byte sync + 4-byte address + user data + error-correction). Raw Mode 1
/// puts user data at offset 16; raw Mode 2 (Form 1, typical for PlayStation)
/// puts it at 24 after an 8-byte subheader. DVD images (PS2 `.iso`, GameCube,
/// Wii, Xbox) are always cooked.
enum Track {
    Cooked,
    RawMode1,
    RawMode2,
}

impl Track {
    /// Bytes per physical sector on disk.
    fn stride(&self) -> u64 {
        match self {
            Track::Cooked => SECTOR,
            _ => 2352,
        }
    }

    /// Byte offset of the 2048-byte user area within a physical sector.
    fn data_offset(&self) -> u64 {
        match self {
            Track::Cooked => 0,
            Track::RawMode1 => 16,
            Track::RawMode2 => 24,
        }
    }
}

/// A disc-image track reader that hides cooked-vs-raw sector layout. It sniffs
/// the layout once on open, then exposes two views: [`read_raw`] for absolute
/// byte offsets (DVD headers and signatures, which ignore CD framing) and
/// [`read_logical`] for the ISO 9660 logical address space (stitching the
/// 2048-byte user areas across raw sectors). All ISO navigation goes through the
/// logical view, so the same record-walking code serves `.iso` and `.bin` alike.
struct SectorReader {
    file: File,
    track: Track,
}

impl SectorReader {
    /// Open `path` and sniff its sector layout from the first bytes: a raw CD
    /// sync pattern selects Mode 1 or Mode 2 by the mode byte at offset 15;
    /// anything else (including files too short to sniff) is treated as cooked.
    fn open(path: &Path) -> Option<SectorReader> {
        let mut file = File::open(path).ok()?;
        let mut head = [0u8; 16];
        let track = match file.read_exact(&mut head) {
            Ok(()) if head[..12] == RAW_SYNC => match head[15] {
                2 => Track::RawMode2,
                _ => Track::RawMode1,
            },
            _ => Track::Cooked,
        };
        Some(SectorReader { file, track })
    }

    /// Read `len` bytes at an absolute byte `offset` in the file. Used for DVD
    /// headers and the Xbox signature, which address the raw image directly.
    fn read_raw(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset)).ok()?;
        let mut buf = vec![0u8; len];
        self.file.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    /// Read `len` bytes from logical sector `lba` onward, transparently skipping
    /// the sync/subheader/ECC framing of raw sectors so the caller sees a clean
    /// ISO 9660 byte stream.
    fn read_logical(&mut self, lba: u32, len: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let sectors = len.div_ceil(SECTOR as usize);
        for i in 0..sectors as u64 {
            let phys = (lba as u64 + i) * self.track.stride() + self.track.data_offset();
            out.extend_from_slice(&self.read_raw(phys, SECTOR as usize)?);
        }
        out.truncate(len);
        Some(out)
    }

    /// Parse the root directory record out of the primary volume descriptor.
    fn root_entry(&mut self) -> Option<Entry> {
        let pvd = self.read_logical(PVD_LBA as u32, SECTOR as usize)?;
        // PVD: type byte 1, standard identifier "CD001", version 1.
        if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
            return None;
        }
        let rec = pvd.get(PVD_ROOT_RECORD..PVD_ROOT_RECORD + 34)?;
        parse_record(rec).map(|(e, _)| e)
    }

    /// Read all records in a directory, skipping the "." and ".." entries.
    fn read_dir(&mut self, dir: &Entry) -> Vec<(String, Entry)> {
        let data = match self.read_logical(dir.lba, dir.len as usize) {
            Some(d) => d,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos < data.len() {
            let rec_len = data[pos] as usize;
            if rec_len == 0 {
                // A zero length means no more records in this sector; records
                // never straddle a sector boundary, so jump to the next one.
                let next = (pos / SECTOR as usize + 1) * SECTOR as usize;
                if next <= pos {
                    break;
                }
                pos = next;
                continue;
            }
            if pos + rec_len > data.len() {
                break;
            }
            if let Some((entry, name)) = parse_record(&data[pos..pos + rec_len]) {
                if name != "." && name != ".." {
                    out.push((name, entry));
                }
            }
            pos += rec_len;
        }
        out
    }

    /// Find a directly-contained entry by (already-normalised) name.
    fn find_in(&mut self, dir: &Entry, name: &str) -> Option<Entry> {
        self.read_dir(dir)
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, e)| e)
    }
}

/// Inspect a disc image and classify it as PSP or PS2 by structure. Returns
/// `None` for anything we can't read or recognise (caller falls back to the
/// extension/filename heuristic).
pub fn classify(path: &Path) -> Option<Disc> {
    let mut r = SectorReader::open(path)?;

    // GameCube and Wii images are raw disc dumps, not ISO 9660 — they have no
    // PVD at sector 16, so they must be recognised by header magic up front or
    // they'd fall through to the filename heuristic and be mislabelled PS2.
    if let Some(disc) = classify_nintendo_disc(&mut r) {
        return Some(disc);
    }

    // Xbox images are XDVDFS, also not ISO 9660 — match by volume signature.
    if is_xbox_disc(&mut r) {
        return Some(Disc::Xbox);
    }

    let root = r.root_entry()?;
    let entries = r.read_dir(&root);

    // PSP UMD: a PSP_GAME directory holding PARAM.SFO.
    if let Some(psp_game) = entries
        .iter()
        .find(|(name, e)| e.is_dir && name == "PSP_GAME")
        .map(|(_, e)| e.clone())
    {
        let title = r
            .find_in(&psp_game, "PARAM.SFO")
            .and_then(|sfo| r.read_logical(sfo.lba, sfo.len as usize))
            .and_then(|bytes| crate::sfo::read_field(&bytes, "TITLE"));
        return Some(Disc::Psp { title });
    }

    // PS2: SYSTEM.CNF at the disc root.
    if entries.iter().any(|(name, e)| !e.is_dir && name == "SYSTEM.CNF") {
        return Some(Disc::Ps2);
    }

    None
}

/// Extract the boot serial (e.g. `SLUS-20946`) from a PlayStation disc image's
/// `SYSTEM.CNF`. That file's `BOOT2 = cdrom0:\SLUS_209.46;1` (PS2) or
/// `BOOT = cdrom:\SCUS_941.63;1` (PS1) line names the boot ELF, whose stem is the
/// disc serial in `LLLL_NNN.NN` form; PCSX2 and DuckStation normalise it to
/// `LLLL-NNNNN` (underscore→dash, dots dropped) to name savestates and memory
/// cards, and we mirror that so discovery can match their files. Handles cooked
/// `.iso`, raw `.bin`, and `.cue` (via its first BINARY track). `.chd` is
/// compressed and returns `None` until a CHD reader lands. `None` also when the
/// image isn't a readable PlayStation disc or the boot line is missing/garbled.
pub fn disc_serial(path: &Path) -> Option<String> {
    let track = resolve_data_track(path)?;
    let mut r = SectorReader::open(&track)?;
    let root = r.root_entry()?;
    let cnf = r.find_in(&root, "SYSTEM.CNF")?;
    let bytes = r.read_logical(cnf.lba, cnf.len as usize)?;
    parse_boot_serial(&String::from_utf8_lossy(&bytes))
}

/// Resolve the path of the image holding the actual filesystem. A bare `.iso`/
/// `.bin` is its own data track; a `.cue` sheet points at one or more `.bin`
/// files and we take the first `FILE … BINARY` (the data track on a normal
/// PlayStation disc); a `.chd` is compressed and unreadable here, so `None`.
fn resolve_data_track(path: &Path) -> Option<PathBuf> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("cue") => parse_cue_first_binary(path),
        Some("chd") => None,
        _ => Some(path.to_path_buf()),
    }
}

/// Pull the first `FILE "name" BINARY` path out of a `.cue` sheet, resolved
/// relative to the sheet's own directory. `None` if the sheet has no FILE line.
fn parse_cue_first_binary(cue: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(cue).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if !line.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("FILE ")) {
            continue;
        }
        // `FILE "Game.bin" BINARY` — prefer the quoted name; fall back to the
        // second whitespace token for unquoted sheets.
        let name = match (line.find('"'), line.rfind('"')) {
            (Some(a), Some(b)) if b > a => &line[a + 1..b],
            _ => line.split_whitespace().nth(1)?,
        };
        let parent = cue.parent().unwrap_or_else(|| Path::new("."));
        return Some(parent.join(name));
    }
    None
}

/// Parse the disc serial out of a `SYSTEM.CNF` body: find the `BOOT2` (PS2) or
/// `BOOT` (PS1) line, take the final path component after the drive prefix, strip
/// the `;1` version suffix, and normalise (`SLUS_209.46` → `SLUS-20946`).
fn parse_boot_serial(text: &str) -> Option<String> {
    for raw in text.lines() {
        let Some((key, val)) = raw.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.eq_ignore_ascii_case("BOOT2") && !key.eq_ignore_ascii_case("BOOT") {
            continue;
        }
        // `cdrom0:\SLUS_209.46;1` → final component after any `:` `\` `/`.
        let tail = val
            .trim()
            .rsplit(|c| c == '\\' || c == '/' || c == ':')
            .next()
            .unwrap_or("");
        let stem = tail.split(';').next().unwrap_or(tail).trim();
        let serial = normalise_serial(stem);
        if !serial.is_empty() {
            return Some(serial);
        }
    }
    None
}

/// `SLUS_209.46` → `SLUS-20946`: drop dots, turn the first underscore into a
/// dash, uppercase the rest.
fn normalise_serial(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut dashed = false;
    for c in raw.chars() {
        match c {
            '.' => {}
            '_' if !dashed => {
                out.push('-');
                dashed = true;
            }
            _ => out.push(c.to_ascii_uppercase()),
        }
    }
    out
}

/// The 6-character game id stamped at the very start of a GameCube/Wii disc
/// header (e.g. `GALE01`). Dolphin keys its `StateSaves/` by this id. Gated on
/// the disc magic so we never return six bytes of garbage for a non-Nintendo
/// image; `None` if the id isn't printable ASCII alphanumerics.
pub fn nintendo_game_id(path: &Path) -> Option<String> {
    let mut r = SectorReader::open(path)?;
    classify_nintendo_disc(&mut r)?;
    let id = r.read_raw(0, 6)?;
    if id.iter().all(u8::is_ascii_alphanumeric) {
        Some(String::from_utf8_lossy(&id).to_ascii_uppercase())
    } else {
        None
    }
}

/// Detect a GameCube or Wii image by its header magic. Wii is checked first
/// because a Wii disc's 0x1C word is zero (only the GameCube field is shared in
/// layout, not value), so the two magics never collide.
fn classify_nintendo_disc(r: &mut SectorReader) -> Option<Disc> {
    let header = r.read_raw(0, 0x20)?;
    let wii = u32::from_be_bytes(header[0x18..0x1C].try_into().ok()?);
    let gamecube = u32::from_be_bytes(header[0x1C..0x20].try_into().ok()?);
    if wii == WII_MAGIC {
        return Some(Disc::Wii { title: disc_name(r) });
    }
    if gamecube == GAMECUBE_MAGIC {
        return Some(Disc::GameCube { title: disc_name(r) });
    }
    None
}

/// Match an original Xbox image by its XDVDFS volume signature at any of the
/// known partition bases.
fn is_xbox_disc(r: &mut SectorReader) -> bool {
    XBOX_SIGNATURE_OFFSETS
        .iter()
        .any(|&off| r.read_raw(off, XBOX_SIGNATURE.len()).as_deref() == Some(XBOX_SIGNATURE))
}

/// Read the null-terminated internal game name from a GameCube/Wii header.
fn disc_name(r: &mut SectorReader) -> Option<String> {
    let raw = r.read_raw(DISC_NAME_OFFSET, DISC_NAME_LEN)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let name = String::from_utf8_lossy(&raw[..end]).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Parse a single directory record, returning the entry and its decoded name.
fn parse_record(rec: &[u8]) -> Option<(Entry, String)> {
    if rec.len() < 33 {
        return None;
    }
    let lba = u32::from_le_bytes(rec[2..6].try_into().ok()?);
    let len = u32::from_le_bytes(rec[10..14].try_into().ok()?);
    let flags = rec[25];
    let id_len = rec[32] as usize;
    let id = rec.get(33..33 + id_len)?;
    Some((
        Entry {
            lba,
            len,
            is_dir: flags & FLAG_DIRECTORY != 0,
        },
        decode_name(id),
    ))
}

/// Decode an ISO 9660 file identifier into an uppercase, version-stripped name.
/// `0x00`/`0x01` are the special "." and ".." entries.
fn decode_name(id: &[u8]) -> String {
    match id {
        [0] => return ".".to_string(),
        [1] => return "..".to_string(),
        _ => {}
    }
    let s = String::from_utf8_lossy(id);
    // Strip the ";1" file-version suffix (files have it; directories don't).
    let s = s.split(';').next().unwrap_or(&s);
    s.trim_end_matches('.').to_ascii_uppercase()
}

#[cfg(test)]
pub(crate) mod testfixtures {
    //! Builders for minimal but valid ISO 9660 images, shared by this module's
    //! tests and the library scan tests.

    const SECTOR: usize = 2048;

    /// A directory record: where the entry's data lives, its size, and type.
    fn record(name: &[u8], lba: u32, len: u32, is_dir: bool) -> Vec<u8> {
        let mut rec_len = 33 + name.len();
        if rec_len % 2 == 1 {
            rec_len += 1; // records are padded to an even length
        }
        let mut r = vec![0u8; rec_len];
        r[0] = rec_len as u8;
        r[2..6].copy_from_slice(&lba.to_le_bytes());
        r[6..10].copy_from_slice(&lba.to_be_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        r[14..18].copy_from_slice(&len.to_be_bytes());
        r[25] = if is_dir { 0x02 } else { 0 };
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r
    }

    /// Pack records into a one-sector directory, prefixed by "." and "..".
    fn directory(self_lba: u32, parent_lba: u32, children: &[Vec<u8>]) -> Vec<u8> {
        let mut sector = vec![0u8; SECTOR];
        let mut pos = 0;
        let mut put = |rec: &[u8], pos: &mut usize| {
            sector[*pos..*pos + rec.len()].copy_from_slice(rec);
            *pos += rec.len();
        };
        put(&record(&[0], self_lba, SECTOR as u32, true), &mut pos);
        put(&record(&[1], parent_lba, SECTOR as u32, true), &mut pos);
        for c in children {
            put(c, &mut pos);
        }
        sector
    }

    /// A primary volume descriptor whose root directory record points at `root_lba`.
    fn pvd(root_lba: u32) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR];
        s[0] = 1;
        s[1..6].copy_from_slice(b"CD001");
        s[6] = 1;
        let root = record(&[0], root_lba, SECTOR as u32, true);
        s[156..156 + root.len()].copy_from_slice(&root);
        s
    }

    /// Lay sectors 0..=last into a flat image, placing `sectors[lba]` at `lba`.
    fn assemble(sectors: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let max = sectors.iter().map(|(lba, _)| *lba).max().unwrap_or(16);
        let mut img = vec![0u8; (max as usize + 1) * SECTOR];
        for (lba, data) in sectors {
            let off = *lba as usize * SECTOR;
            img[off..off + data.len()].copy_from_slice(data);
        }
        img
    }

    /// Minimal single-field PARAM.SFO carrying `TITLE`.
    fn param_sfo(title: &str) -> Vec<u8> {
        let key = b"TITLE\0";
        let mut value = title.as_bytes().to_vec();
        value.push(0);
        let key_table_start = 20 + 16;
        let data_table_start = key_table_start + key.len();
        let mut out = Vec::new();
        out.extend_from_slice(b"\x00PSF");
        out.extend_from_slice(&0x0101_0000u32.to_le_bytes());
        out.extend_from_slice(&(key_table_start as u32).to_le_bytes());
        out.extend_from_slice(&(data_table_start as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x0204u16.to_le_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&value);
        out
    }

    /// A PSP UMD image: root → PSP_GAME → PARAM.SFO(title).
    pub fn psp_iso(title: &str) -> Vec<u8> {
        // LBAs: 16 PVD, 18 root dir, 19 PSP_GAME dir, 20 PARAM.SFO data.
        let sfo = param_sfo(title);
        let root = directory(18, 18, &[record(b"PSP_GAME", 19, SECTOR as u32, true)]);
        let psp_game = directory(19, 18, &[record(b"PARAM.SFO;1", 20, sfo.len() as u32, false)]);
        assemble(&[(16, pvd(18)), (18, root), (19, psp_game), (20, sfo)])
    }

    /// A PS2 image: root → SYSTEM.CNF.
    pub fn ps2_iso() -> Vec<u8> {
        let cnf = b"BOOT2 = cdrom0:\\SLUS_209.46;1\n".to_vec();
        let root = directory(18, 18, &[record(b"SYSTEM.CNF;1", 19, cnf.len() as u32, false)]);
        assemble(&[(16, pvd(18)), (18, root), (19, cnf)])
    }

    /// A PS1 image: root → SYSTEM.CNF whose `BOOT` line names the boot ELF.
    pub fn ps1_iso() -> Vec<u8> {
        let cnf = b"BOOT = cdrom:\\SCUS_941.63;1\n".to_vec();
        let root = directory(18, 18, &[record(b"SYSTEM.CNF;1", 19, cnf.len() as u32, false)]);
        assemble(&[(16, pvd(18)), (18, root), (19, cnf)])
    }

    /// Re-frame a cooked image's 2048-byte logical sectors as raw 2352-byte
    /// Mode 2 (Form 1) sectors: 12-byte sync, 4-byte address (mode byte = 2),
    /// 8-byte subheader, then the 2048-byte user area at offset 24. This is the
    /// layout DuckStation reads from a PlayStation `.bin`.
    pub fn raw_mode2(cooked: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in cooked.chunks(SECTOR) {
            let mut sector = vec![0u8; 2352];
            sector[0] = 0x00;
            for b in &mut sector[1..11] {
                *b = 0xFF;
            }
            sector[11] = 0x00;
            sector[15] = 2; // mode byte
            sector[24..24 + chunk.len()].copy_from_slice(chunk);
            out.extend_from_slice(&sector);
        }
        out
    }

    /// A raw GameCube disc image: magic 0xC2339F3D at 0x1C, name at 0x20.
    pub fn gamecube_iso(title: &str) -> Vec<u8> {
        nintendo_disc(0x1C, 0xC233_9F3D, title)
    }

    /// A raw Wii disc image: magic 0x5D1C9EA3 at 0x18, name at 0x20.
    pub fn wii_iso(title: &str) -> Vec<u8> {
        nintendo_disc(0x18, 0x5D1C_9EA3, title)
    }

    /// One header sector: a 6-char game id at offset 0, a single magic word, and
    /// an internal game name at 0x20.
    fn nintendo_disc(magic_offset: usize, magic: u32, title: &str) -> Vec<u8> {
        let mut img = vec![0u8; SECTOR];
        img[0..6].copy_from_slice(b"GALE01");
        img[magic_offset..magic_offset + 4].copy_from_slice(&magic.to_be_bytes());
        let name = title.as_bytes();
        img[0x20..0x20 + name.len()].copy_from_slice(name);
        img
    }

    /// A raw (extracted-XISO) Xbox image: signature at sector 32 (0x10000).
    pub fn xbox_iso() -> Vec<u8> {
        let sig = b"MICROSOFT*XBOX*MEDIA";
        let mut img = vec![0u8; 0x10000 + sig.len()];
        img[0x10000..0x10000 + sig.len()].copy_from_slice(sig);
        img
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
    fn detects_psp_umd_and_reads_title() {
        let f = write_temp(&testfixtures::psp_iso("Ace Combat X: Skies of Deception"));
        match classify(f.path()) {
            Some(Disc::Psp { title }) => {
                assert_eq!(title.as_deref(), Some("Ace Combat X: Skies of Deception"));
            }
            _ => panic!("expected a PSP disc"),
        }
    }

    #[test]
    fn detects_ps2_disc() {
        let f = write_temp(&testfixtures::ps2_iso());
        assert!(matches!(classify(f.path()), Some(Disc::Ps2)));
    }

    #[test]
    fn detects_gamecube_disc_and_reads_name() {
        let f = write_temp(&testfixtures::gamecube_iso("LUIGIS MANSION"));
        match classify(f.path()) {
            Some(Disc::GameCube { title }) => {
                assert_eq!(title.as_deref(), Some("LUIGIS MANSION"));
            }
            _ => panic!("expected a GameCube disc"),
        }
    }

    #[test]
    fn detects_wii_disc_and_reads_name() {
        let f = write_temp(&testfixtures::wii_iso("SUPER MARIO GALAXY"));
        match classify(f.path()) {
            Some(Disc::Wii { title }) => {
                assert_eq!(title.as_deref(), Some("SUPER MARIO GALAXY"));
            }
            _ => panic!("expected a Wii disc"),
        }
    }

    #[test]
    fn detects_xbox_disc() {
        let f = write_temp(&testfixtures::xbox_iso());
        assert!(matches!(classify(f.path()), Some(Disc::Xbox)));
    }

    #[test]
    fn non_iso_bytes_classify_as_none() {
        let f = write_temp(b"not an iso at all, just some bytes");
        assert!(classify(f.path()).is_none());
    }

    #[test]
    fn extracts_ps2_serial_from_system_cnf() {
        // The ps2_iso fixture's SYSTEM.CNF boots `cdrom0:\SLUS_209.46;1`.
        let f = write_temp(&testfixtures::ps2_iso());
        assert_eq!(disc_serial(f.path()).as_deref(), Some("SLUS-20946"));
    }

    #[test]
    fn extracts_ps1_serial_from_raw_2352_bin() {
        // A PS1 `.bin` is raw Mode 2: the same filesystem, reframed into
        // 2352-byte sectors. The SectorReader must stitch the user areas back
        // together to reach SYSTEM.CNF and read `cdrom:\SCUS_941.63;1`.
        let raw = testfixtures::raw_mode2(&testfixtures::ps1_iso());
        let mut f = tempfile::Builder::new().suffix(".bin").tempfile().unwrap();
        f.write_all(&raw).unwrap();
        f.flush().unwrap();
        assert_eq!(disc_serial(f.path()).as_deref(), Some("SCUS-94163"));
    }

    #[test]
    fn follows_cue_sheet_to_its_first_binary_track() {
        // A `.cue` points at the `.bin` holding the filesystem; disc_serial must
        // resolve the sheet to that track and read the serial from it.
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("Game.bin");
        std::fs::write(&bin, testfixtures::raw_mode2(&testfixtures::ps1_iso())).unwrap();
        let cue = dir.path().join("Game.cue");
        std::fs::write(
            &cue,
            "FILE \"Game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        assert_eq!(disc_serial(&cue).as_deref(), Some("SCUS-94163"));
    }

    #[test]
    fn chd_is_not_yet_readable() {
        // `.chd` is compressed; until a CHD reader lands we return None rather
        // than misread the container as a raw track.
        let f = tempfile::Builder::new().suffix(".chd").tempfile().unwrap();
        assert!(disc_serial(f.path()).is_none());
    }

    #[test]
    fn parses_boot_serial_variants() {
        // PS2 BOOT2 with the usual dotted form.
        assert_eq!(
            parse_boot_serial("BOOT2 = cdrom0:\\SLUS_209.46;1\n").as_deref(),
            Some("SLUS-20946")
        );
        // PS1 BOOT line, forward-slash drive prefix, lowercase key.
        assert_eq!(
            parse_boot_serial("boot = cdrom:\\SCUS_941.63;1").as_deref(),
            Some("SCUS-94163")
        );
        // No boot line at all.
        assert!(parse_boot_serial("VER = 1.00\nVMODE = NTSC\n").is_none());
    }

    #[test]
    fn reads_nintendo_game_id_only_for_real_discs() {
        let gc = write_temp(&testfixtures::gamecube_iso("LUIGIS MANSION"));
        assert_eq!(nintendo_game_id(gc.path()).as_deref(), Some("GALE01"));
        // A PS2 image has no Nintendo magic, so no id is returned.
        let ps2 = write_temp(&testfixtures::ps2_iso());
        assert!(nintendo_game_id(ps2.path()).is_none());
    }
}
