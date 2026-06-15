//! Canonical platform catalogue and ROM-extension routing.
//!
//! Arcadia indexes files the user already has; it never downloads ROMs. The
//! extension map is how a bare file on disk becomes a `Game` on a `platform`.

/// A platform Arcadia understands. `adapter_hint` is the adapter id we prefer
/// when launching this platform, if such an emulator is installed.
pub struct PlatformDef {
    pub id: &'static str,
    pub name: &'static str,
    pub manufacturer: &'static str,
    pub generation: i64,
    /// Lowercase file extensions (no dot) that map to this platform.
    pub extensions: &'static [&'static str],
    /// Preferred adapter id for launching.
    pub adapter_hint: &'static str,
}

/// The MVP platform catalogue. Spans 2D retro through PS3, matching the
/// initial adapter set (RetroArch + six standalone emulators).
pub const PLATFORMS: &[PlatformDef] = &[
    PlatformDef {
        id: "nes",
        name: "Nintendo Entertainment System",
        manufacturer: "Nintendo",
        generation: 3,
        extensions: &["nes", "fds", "unf"],
        adapter_hint: "mesen",
    },
    PlatformDef {
        id: "snes",
        name: "Super Nintendo",
        manufacturer: "Nintendo",
        generation: 4,
        extensions: &["sfc", "smc", "swc", "fig"],
        adapter_hint: "snes9x",
    },
    PlatformDef {
        id: "n64",
        name: "Nintendo 64",
        manufacturer: "Nintendo",
        generation: 5,
        extensions: &["n64", "z64", "v64"],
        adapter_hint: "mupen64plus",
    },
    PlatformDef {
        id: "gamecube",
        name: "Nintendo GameCube",
        manufacturer: "Nintendo",
        generation: 6,
        extensions: &["gcm", "gcz", "rvz"],
        adapter_hint: "dolphin",
    },
    PlatformDef {
        id: "wii",
        name: "Nintendo Wii",
        manufacturer: "Nintendo",
        generation: 7,
        extensions: &["wbfs", "wad"],
        adapter_hint: "dolphin",
    },
    PlatformDef {
        id: "gb",
        name: "Game Boy",
        manufacturer: "Nintendo",
        generation: 4,
        extensions: &["gb"],
        adapter_hint: "mgba",
    },
    PlatformDef {
        id: "gbc",
        name: "Game Boy Color",
        manufacturer: "Nintendo",
        generation: 5,
        extensions: &["gbc"],
        adapter_hint: "mgba",
    },
    PlatformDef {
        id: "gba",
        name: "Game Boy Advance",
        manufacturer: "Nintendo",
        generation: 6,
        extensions: &["gba"],
        adapter_hint: "mgba",
    },
    PlatformDef {
        id: "nds",
        name: "Nintendo DS",
        manufacturer: "Nintendo",
        generation: 7,
        extensions: &["nds"],
        adapter_hint: "melonds",
    },
    PlatformDef {
        id: "genesis",
        name: "Sega Genesis / Mega Drive",
        manufacturer: "Sega",
        generation: 4,
        extensions: &["md", "gen", "smd"],
        adapter_hint: "blastem",
    },
    PlatformDef {
        id: "ps1",
        name: "Sony PlayStation",
        manufacturer: "Sony",
        generation: 5,
        extensions: &["pbp"],
        adapter_hint: "duckstation",
    },
    PlatformDef {
        id: "ps2",
        name: "Sony PlayStation 2",
        manufacturer: "Sony",
        generation: 6,
        extensions: &["iso", "bin", "chd", "cso"],
        adapter_hint: "pcsx2",
    },
    PlatformDef {
        id: "psp",
        name: "Sony PSP",
        manufacturer: "Sony",
        generation: 7,
        extensions: &["cso"],
        adapter_hint: "ppsspp",
    },
    PlatformDef {
        id: "ps3",
        // PS3 titles are disc-folder dumps (PS3_GAME/PARAM.SFO) or ISOs, not a
        // single file extension — the scanner detects them structurally. `.pkg`
        // is deliberately absent: loose PKGs in a library are updates, DLC, and
        // PSN-license blobs, not bootable games.
        name: "Sony PlayStation 3",
        manufacturer: "Sony",
        generation: 7,
        extensions: &[],
        adapter_hint: "rpcs3",
    },
    PlatformDef {
        id: "vita",
        // Vita titles are folder dumps (`.vpk` installers or extracted apps);
        // the scanner detects `.vpk` and Vita3K manages installed games.
        name: "Sony PlayStation Vita",
        manufacturer: "Sony",
        generation: 8,
        extensions: &["vpk"],
        adapter_hint: "vita3k",
    },
    // ---- Sega ------------------------------------------------------------
    PlatformDef {
        id: "sms",
        name: "Sega Master System",
        manufacturer: "Sega",
        generation: 3,
        extensions: &["sms"],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        id: "gamegear",
        name: "Sega Game Gear",
        manufacturer: "Sega",
        generation: 4,
        extensions: &["gg"],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        id: "sega32x",
        name: "Sega 32X",
        manufacturer: "Sega",
        generation: 4,
        extensions: &["32x"],
        adapter_hint: "retroarch",
    },
    PlatformDef {
        // Sega CD / Mega-CD discs are `.cue`/`.chd` — shared CD extensions the
        // scanner can't cheaply pin to this platform, so none are mapped here.
        id: "segacd",
        name: "Sega CD / Mega-CD",
        manufacturer: "Sega",
        generation: 4,
        extensions: &[],
        adapter_hint: "retroarch",
    },
    PlatformDef {
        // Saturn discs are `.cue`/`.chd`/`.iso`; structural sniffing (a `SEGA
        // SEGASATURN` signature) routes them, so no bare extension is claimed.
        id: "saturn",
        name: "Sega Saturn",
        manufacturer: "Sega",
        generation: 5,
        extensions: &[],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        // `.gdi`/`.cdi` are Dreamcast-specific; `.chd` is shared and routed
        // structurally.
        id: "dreamcast",
        name: "Sega Dreamcast",
        manufacturer: "Sega",
        generation: 6,
        extensions: &["gdi", "cdi"],
        adapter_hint: "flycast",
    },
    // ---- NEC -------------------------------------------------------------
    PlatformDef {
        id: "pcengine",
        name: "NEC PC Engine / TurboGrafx-16",
        manufacturer: "NEC",
        generation: 4,
        extensions: &["pce", "sgx"],
        adapter_hint: "mednafen",
    },
    // ---- Nintendo (additional) ------------------------------------------
    PlatformDef {
        id: "virtualboy",
        name: "Nintendo Virtual Boy",
        manufacturer: "Nintendo",
        generation: 5,
        extensions: &["vb", "vboy"],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        id: "n3ds",
        name: "Nintendo 3DS",
        manufacturer: "Nintendo",
        generation: 8,
        extensions: &["3ds", "cci", "cxi", "cia"],
        adapter_hint: "lime3ds",
    },
    PlatformDef {
        id: "switch",
        name: "Nintendo Switch",
        manufacturer: "Nintendo",
        generation: 8,
        extensions: &["nsp", "xci"],
        adapter_hint: "ryujinx",
    },
    PlatformDef {
        id: "wiiu",
        name: "Nintendo Wii U",
        manufacturer: "Nintendo",
        generation: 8,
        extensions: &["wud", "wux", "wua", "rpx"],
        adapter_hint: "cemu",
    },
    // ---- Microsoft -------------------------------------------------------
    PlatformDef {
        // Original Xbox dumps are `.iso` (XDVDFS), shared with PS2 — routed by a
        // structural `MICROSOFT*XBOX*MEDIA` signature, so no extension is claimed.
        id: "xbox",
        name: "Microsoft Xbox",
        manufacturer: "Microsoft",
        generation: 6,
        extensions: &[],
        adapter_hint: "xemu",
    },
    // ---- Atari -----------------------------------------------------------
    PlatformDef {
        id: "atari2600",
        name: "Atari 2600",
        manufacturer: "Atari",
        generation: 2,
        extensions: &["a26"],
        adapter_hint: "stella",
    },
    PlatformDef {
        id: "atari5200",
        name: "Atari 5200",
        manufacturer: "Atari",
        generation: 2,
        extensions: &["a52"],
        adapter_hint: "atari800",
    },
    PlatformDef {
        id: "atari7800",
        name: "Atari 7800",
        manufacturer: "Atari",
        generation: 3,
        extensions: &["a78"],
        adapter_hint: "retroarch",
    },
    PlatformDef {
        id: "lynx",
        name: "Atari Lynx",
        manufacturer: "Atari",
        generation: 4,
        extensions: &["lnx"],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        id: "jaguar",
        name: "Atari Jaguar",
        manufacturer: "Atari",
        generation: 5,
        extensions: &["j64", "jag"],
        adapter_hint: "bigpemu",
    },
    // ---- Other handhelds & micros ---------------------------------------
    PlatformDef {
        id: "wonderswan",
        name: "Bandai WonderSwan",
        manufacturer: "Bandai",
        generation: 5,
        extensions: &["ws", "wsc"],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        id: "ngp",
        name: "SNK Neo Geo Pocket",
        manufacturer: "SNK",
        generation: 5,
        extensions: &["ngp", "ngc"],
        adapter_hint: "mednafen",
    },
    PlatformDef {
        id: "colecovision",
        name: "ColecoVision",
        manufacturer: "Coleco",
        generation: 2,
        extensions: &["col"],
        adapter_hint: "gearcoleco",
    },
    PlatformDef {
        id: "c64",
        name: "Commodore 64",
        manufacturer: "Commodore",
        generation: 2,
        extensions: &["d64", "t64", "crt", "prg"],
        adapter_hint: "vice",
    },
    PlatformDef {
        id: "amiga",
        name: "Commodore Amiga",
        manufacturer: "Commodore",
        generation: 4,
        extensions: &["adf", "adz", "ipf", "hdf"],
        adapter_hint: "retroarch",
    },
    PlatformDef {
        id: "msx",
        name: "MSX",
        manufacturer: "ASCII / Microsoft",
        generation: 3,
        extensions: &["mx1", "mx2"],
        adapter_hint: "openmsx",
    },
    // ---- Arcade ----------------------------------------------------------
    PlatformDef {
        // Arcade romsets are `.zip`/`.7z` of many small files; the archive scan
        // routes them by MAME/FBNeo set name, so no bare extension is claimed.
        id: "arcade",
        name: "Arcade (MAME / FinalBurn Neo)",
        manufacturer: "Various",
        generation: 0,
        extensions: &[],
        adapter_hint: "mame",
    },
];

/// Look up a platform definition by slug.
pub fn by_id(id: &str) -> Option<&'static PlatformDef> {
    PLATFORMS.iter().find(|p| p.id == id)
}

/// Resolve a ROM to a platform from its filename stem and extension alone.
///
/// This is the no-I/O fallback. The disc extensions `.iso` and `.cso` are shared
/// across Sony generations, so we disambiguate by the Sony title ID embedded in
/// many filenames:
///   - `.iso`: PS3 (`B…`/`NP…`) or PSP (`UL…`/`UC…`), else PS2.
///   - `.cso`: PS2 (`SL…`/`SC…`) or, by default, PSP.
///
/// The scanner additionally reads `.iso` volumes structurally (see
/// `crate::iso9660`) for cases where the filename carries no ID — that takes
/// precedence over this heuristic.
pub fn resolve_platform(stem: &str, ext: &str) -> Option<&'static str> {
    match ext {
        "iso" => Some(if contains_ps3_title_id(stem) {
            "ps3"
        } else if contains_psp_title_id(stem) {
            "psp"
        } else {
            "ps2"
        }),
        "cso" => Some(if contains_ps2_title_id(stem) { "ps2" } else { "psp" }),
        _ => platform_for_extension(ext),
    }
}

/// A resolved ROM: the platform it belongs to and, when we could read it
/// cheaply (a PSP UMD's `PARAM.SFO`), its real title.
pub struct RomMatch {
    pub platform: &'static str,
    pub title: Option<String>,
}

/// Resolve a ROM **on disk**, reading disc contents when the filename alone is
/// ambiguous. This is the routing the scanner uses; it strictly improves on
/// [`resolve_platform`].
///
/// For `.iso` we disambiguate in priority order:
///   1. a PS3 Sony title ID in the filename (`BLES02166`, `NPUB30584`, …);
///   2. the disc structure — a GameCube/Wii dump carries a header magic, a PSP
///      UMD has `PSP_GAME/PARAM.SFO`, a PS2 disc has `SYSTEM.CNF` (the volume
///      also yields the GameCube/Wii/PSP game's real title);
///   3. failing both (image unreadable / unrecognised), the filename heuristic.
///
/// `.bin` and `.cue` are also disambiguated on disk:
///   * a `.bin` paired with a same-stem `.cue`, or named like a `(Track NN)`
///     split, is a CD *track* — the `.cue` is the game, so the track is skipped;
///   * a loose `.bin` carrying a `SEGA` console signature is a Genesis cartridge;
///   * a `.cue` is sniffed through to its data track, whose signature tells PS1
///     from Sega CD / Saturn.
///
/// Compressed (`.cso`/`.chd`) images are still routed by the filename heuristic —
/// we don't decompress them.
pub fn resolve_rom(path: &std::path::Path, stem: &str, ext: &str) -> Option<RomMatch> {
    let guess = classify_content(path, stem, ext)?;
    // A precise content signal — a platform-specific extension, a disc read
    // structurally, a Sony title ID in the name — is trusted as-is. An *ambiguous*
    // guess (a shared disc extension's default, an un-inspectable archive) instead
    // defers to how the user filed the ROM: the system folder it lives under. That
    // folder is what rescues a `.7z`-compressed PS2 disc or a PS3 folder-dump from
    // landing in "arcade".
    let platform = if guess.precise {
        guess.platform
    } else {
        folder_platform_hint(path).unwrap_or(guess.platform)
    };
    Some(RomMatch { platform, title: guess.title })
}

/// A content-derived platform guess and how much to trust it. `precise` means a
/// confident signal (a specific extension, a structural disc read, a Sony title
/// ID); when `false` the platform is only a shared-extension / arcade default
/// that [`folder_platform_hint`] may override.
struct Guess {
    platform: &'static str,
    title: Option<String>,
    precise: bool,
}

impl Guess {
    fn precise(platform: &'static str, title: Option<String>) -> Self {
        Guess { platform, title, precise: true }
    }
    fn ambiguous(platform: &'static str) -> Self {
        Guess { platform, title: None, precise: false }
    }
}

/// Resolve a ROM from its contents and name alone (no folder context). Mirrors
/// the old `resolve_rom` body but tags each result precise/ambiguous so the
/// caller knows whether the folder hint is allowed to override it.
fn classify_content(path: &std::path::Path, stem: &str, ext: &str) -> Option<Guess> {
    match ext {
        "iso" if !contains_ps3_title_id(stem) => {
            match crate::iso9660::classify(path) {
                Some(crate::iso9660::Disc::Psp { title }) => {
                    return Some(Guess::precise("psp", title))
                }
                Some(crate::iso9660::Disc::Ps2) => return Some(Guess::precise("ps2", None)),
                Some(crate::iso9660::Disc::GameCube { title }) => {
                    return Some(Guess::precise("gamecube", title))
                }
                Some(crate::iso9660::Disc::Wii { title }) => {
                    return Some(Guess::precise("wii", title))
                }
                Some(crate::iso9660::Disc::Xbox) => return Some(Guess::precise("xbox", None)),
                None => {} // unreadable — fall through to the filename heuristic
            }
        }
        "bin" => return classify_bin(path, stem),
        "cue" => {
            return Some(match cue_target_platform(path) {
                Some(platform) => Guess::precise(platform, None),
                None => Guess::ambiguous("ps1"),
            });
        }
        "zip" => {
            return Some(classify_archive(&crate::archive::entry_names(path).unwrap_or_default()))
        }
        // `.7z` is inspected the same way: its header lists the inner files, which
        // route a compressed disc/cartridge image just like a loose file would.
        "7z" => {
            return Some(classify_archive(
                &crate::archive::entry_names_7z(path).unwrap_or_default(),
            ))
        }
        _ => {}
    }
    classify_by_name(stem, ext)
}

/// Resolve from filename + extension only, tagging precision. Specific cartridge
/// extensions and Sony title IDs are confident; the shared disc extensions
/// (`.iso`/`.bin`/`.chd`/…) fall back to their most-common console as an
/// *ambiguous* default a folder hint may correct.
fn classify_by_name(stem: &str, ext: &str) -> Option<Guess> {
    match ext {
        "iso" => {
            if contains_ps3_title_id(stem) {
                return Some(Guess::precise("ps3", None));
            }
            if contains_psp_title_id(stem) {
                return Some(Guess::precise("psp", None));
            }
            return Some(Guess::ambiguous("ps2"));
        }
        "cso" => {
            if contains_ps2_title_id(stem) {
                return Some(Guess::precise("ps2", None));
            }
            return Some(Guess::ambiguous("psp"));
        }
        "bin" => return Some(Guess::ambiguous("ps2")),
        "chd" | "cue" | "m3u" | "img" => return Some(Guess::ambiguous("ps1")),
        _ => {}
    }
    platform_for_extension(ext).map(|p| Guess::precise(p, None))
}

/// Disc-image descriptors that, inside an archive, mark a *console* game rather
/// than an arcade romset. `.bin` is excluded: an arcade romset is often many
/// `.bin` chip dumps, so a bare `.bin` isn't a reliable disc signal — a real
/// disc rip carries a `.cue`/`.iso`/etc. The order is the boot preference (a
/// sheet/playlist describes the raw image it points at, so it wins).
const ARCHIVED_DISC_EXTS: &[&str] = &["m3u", "cue", "gdi", "cdi", "iso", "chd", "cso", "img"];

/// Classify an archive from its entry names (works for `.zip` and `.7z` alike).
/// Three cases, in order:
///   1. a single disc image inside (a PS1 `.cue`+`.bin`, a PS2/PSP `.iso`, a
///      Dreamcast `.gdi`, …) is a console game, routed by its *inner* filename
///      exactly as a loose file of that name would route — and carrying that
///      name's precision (a Sony title ID is confident, a bare `.iso` is not);
///   2. a single inner cartridge ROM identifies its console directly (precise);
///   3. anything else (many unrecognised files, or an archive we couldn't read,
///      which yields an empty name list) is an *ambiguous* arcade default the
///      folder hint may override.
fn classify_archive(names: &[String]) -> Guess {
    // (1) A disc image inside => a console game. Route it by the inner file's
    // name, which carries the same Sony title-ID / extension signal we use for
    // loose discs (so a zipped `Daxter [ULUS10041].iso` is PSP, a `.cue` is PS1).
    if let Some(inner) = pick_archived_disc(names) {
        let p = std::path::Path::new(&inner);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if let Some(g) = classify_by_name(stem, &ext) {
            return g;
        }
    }

    // (2) Cartridge vs. romset, by the inner extensions.
    let mut platforms = std::collections::BTreeSet::new();
    for name in names {
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if let Some(p) = ext.as_deref().and_then(cartridge_platform_for_extension) {
            platforms.insert(p);
        }
    }
    if platforms.len() == 1 {
        return Guess::precise(platforms.into_iter().next().unwrap(), None);
    }

    // (3) No recognised ROM, mixed contents, or an unreadable archive => arcade,
    // but only as a default the folder hint can correct.
    Guess::ambiguous("arcade")
}

/// The representative disc descriptor in an archive, preferring a sheet/playlist
/// over the raw image it references. `None` if the archive holds no disc image.
fn pick_archived_disc(names: &[String]) -> Option<String> {
    for ext in ARCHIVED_DISC_EXTS {
        if let Some(found) = names.iter().find(|n| {
            std::path::Path::new(n)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        }) {
            return Some(found.clone());
        }
    }
    None
}

/// Like [`platform_for_extension`] but only for *unambiguous cartridge*
/// extensions — the shared disc/archive formats return `None` so they can't be
/// mistaken for a platform signal when found inside an archive.
fn cartridge_platform_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "iso" | "bin" | "chd" | "cso" | "cue" | "m3u" | "img" | "zip" | "7z" => None,
        _ => platform_for_extension(ext),
    }
}

/// Classify a loose `.bin`. Returns `None` for CD tracks (indexed via their
/// `.cue`), Genesis for cartridge dumps (precise), else the shared-extension
/// default (ambiguous, so a folder hint may correct it).
fn classify_bin(path: &std::path::Path, stem: &str) -> Option<Guess> {
    if is_cd_track(path, stem) {
        return None;
    }
    if is_genesis_bin(path) {
        return Some(Guess::precise("genesis", None));
    }
    Some(Guess::ambiguous("ps2"))
}

/// A `.bin` is a CD data track (not a standalone game) when a sibling `.cue`
/// shares its stem, or when its name carries a `(Track NN)` split marker.
fn is_cd_track(path: &std::path::Path, stem: &str) -> bool {
    if path.with_extension("cue").is_file() {
        return true;
    }
    let lower = stem.to_ascii_lowercase();
    lower.contains("(track ") || lower.contains(" track ")
}

/// Genesis / Mega Drive cartridge dumps stamp `SEGA` at ROM offset 0x100.
fn is_genesis_bin(path: &std::path::Path) -> bool {
    read_at(path, 0x100, 4).as_deref() == Some(b"SEGA")
}

/// Sniff a `.cue` through to its first data track and identify the console from
/// the data's signature. `None` when unreadable (caller defaults to PS1).
fn cue_target_platform(path: &std::path::Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(path).ok()?;
    // `FILE "Some Game (Track 1).bin" BINARY` — take the first quoted name.
    let file = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("FILE "))
        .and_then(|rest| rest.split('"').nth(1))?;
    let data = path.parent()?.join(file);
    let head = read_at(&data, 0, 0x2_0000)?;
    if contains(&head, b"SEGADISCSYSTEM") {
        Some("segacd")
    } else if contains(&head, b"SEGA SEGASATURN") {
        Some("saturn")
    } else if contains(&head, b"PLAYSTATION") {
        Some("ps1")
    } else {
        None
    }
}

/// Read up to `len` bytes at `offset` from a file (fewer if it ends sooner).
fn read_at(path: &std::path::Path, offset: u64, len: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = Vec::new();
    f.take(len as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Naive substring search over bytes.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Whether `name` contains a Sony title ID — four uppercase letters followed by
/// five digits — whose first bytes satisfy `prefix`.
fn contains_title_id(name: &str, prefix: impl Fn(&[u8]) -> bool) -> bool {
    name.as_bytes().windows(9).any(|w| {
        w[..4].iter().all(u8::is_ascii_uppercase)
            && w[4..].iter().all(u8::is_ascii_digit)
            && prefix(w)
    })
}

/// PS3 title ID: disc IDs start with `B` (BLES/BLUS/BCES/BCUS/…) and PSN IDs
/// start with `NP`. PS2 and PSP IDs don't, so this won't claim them.
pub fn contains_ps3_title_id(name: &str) -> bool {
    contains_title_id(name, |w| w[0] == b'B' || (w[0] == b'N' && w[1] == b'P'))
}

/// PSP UMD disc ID: starts `UL`/`UC` (ULUS, ULES, UCUS, UCES, ULJM, …). Distinct
/// from PS2 (`SL`/`SC`) and PS3 (`B…`/`NP…`).
pub fn contains_psp_title_id(name: &str) -> bool {
    contains_title_id(name, |w| w[0] == b'U' && (w[1] == b'C' || w[1] == b'L'))
}

/// PS2 disc ID: starts `SL`/`SC` (SLUS, SCES, SLPS, …).
pub fn contains_ps2_title_id(name: &str) -> bool {
    contains_title_id(name, |w| w[0] == b'S' && (w[1] == b'C' || w[1] == b'L'))
}

/// Resolve a lowercase extension (no dot) to a platform id.
///
/// Several platforms share generic extensions (`.iso`, `.bin`, `.cso`). For
/// those we pick the most common disc-based console; the user can re-assign a
/// game's platform in Studio mode. Specific extensions always win.
pub fn platform_for_extension(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    // Disambiguate shared extensions up front. `.iso`/`.bin` default to PS2 but
    // are refined structurally by `resolve_rom`; `.chd`/`.cue`/`.m3u`/`.img` are
    // CD formats whose dominant use in a library is PS1 (PS2 dumps are `.iso`).
    match ext.as_str() {
        "iso" | "bin" => return Some("ps2"),
        "chd" | "cue" | "m3u" | "img" => return Some("ps1"),
        "cso" => return Some("psp"),
        _ => {}
    }
    for p in PLATFORMS {
        if p.extensions.contains(&ext.as_str()) {
            return Some(p.id);
        }
    }
    None
}

/// Infer a platform from the directory path a ROM lives under — the system folder
/// the user files it in (`…/Sony/PS2/Games/…`, `…/Nintendo/SNES/…`). This is the
/// signal of last resort: [`resolve_rom`] only consults it for cases the file's
/// own contents leave ambiguous (a shared `.iso`/`.chd`, an un-inspectable or
/// header-only archive). Each directory component is normalised to lowercase
/// alphanumerics and matched against known folder aliases; the deepest (most
/// specific) match wins, and the filename itself is never inspected.
pub fn folder_platform_hint(path: &std::path::Path) -> Option<&'static str> {
    let dir = path.parent().unwrap_or(path);
    let mut hint = None;
    for comp in dir.components() {
        if let std::path::Component::Normal(s) = comp {
            let norm: String = s
                .to_string_lossy()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if let Some(p) = folder_alias(&norm) {
                hint = Some(p);
            }
        }
    }
    hint
}

/// Map a normalised folder name (lowercase, alphanumerics only) to a platform.
/// Only unambiguous aliases are listed: a combined folder like `Wii_GC`
/// (`wiigc`) deliberately has no entry, leaving such cases to content detection.
fn folder_alias(norm: &str) -> Option<&'static str> {
    Some(match norm {
        "ps1" | "psx" | "psone" | "playstation" | "playstation1" => "ps1",
        "ps2" | "playstation2" => "ps2",
        "ps3" | "playstation3" => "ps3",
        "psp" | "playstationportable" => "psp",
        "vita" | "psvita" | "playstationvita" => "vita",
        "nes" | "famicom" => "nes",
        "snes" | "sfc" | "superfamicom" | "supernintendo" | "supernes" => "snes",
        "n64" | "nintendo64" => "n64",
        "gamecube" | "gcn" | "ngc" => "gamecube",
        "wii" => "wii",
        "wiiu" => "wiiu",
        "gb" | "gameboy" => "gb",
        "gbc" | "gameboycolor" => "gbc",
        "gba" | "gameboyadvance" => "gba",
        "nds" | "nintendods" => "nds",
        "3ds" | "n3ds" | "nintendo3ds" => "n3ds",
        "switch" | "nintendoswitch" => "switch",
        "genesis" | "megadrive" | "segagenesis" | "segamegadrive" => "genesis",
        "mastersystem" | "sms" | "segamastersystem" => "sms",
        "gamegear" => "gamegear",
        "saturn" | "segasaturn" => "saturn",
        "dreamcast" | "segadreamcast" => "dreamcast",
        "segacd" | "megacd" => "segacd",
        "sega32x" | "32x" => "sega32x",
        "pcengine" | "turbografx" | "turbografx16" | "tg16" => "pcengine",
        "xbox" => "xbox",
        "atari2600" => "atari2600",
        "atari5200" => "atari5200",
        "atari7800" => "atari7800",
        "jaguar" | "atarijaguar" => "jaguar",
        "lynx" | "atarilynx" => "lynx",
        "arcade" | "mame" | "fbneo" | "fba" | "neogeo" => "arcade",
        _ => return None,
    })
}

/// All recognised ROM extensions, for fast scan-time filtering.
pub fn all_extensions() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = PLATFORMS.iter().flat_map(|p| p.extensions.iter().copied()).collect();
    v.extend_from_slice(&["iso", "bin", "chd", "cso", "cue", "m3u", "img", "zip", "7z"]);
    v.sort_unstable();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps3_isos_detected_by_title_id() {
        assert_eq!(resolve_platform("BCES00081-[Killzone(R)2]", "iso"), Some("ps3"));
        assert_eq!(resolve_platform("Skate 3 [BLUS30464]", "iso"), Some("ps3"));
        assert_eq!(resolve_platform("skate. [BLUS30059]", "iso"), Some("ps3"));
        assert!(contains_ps3_title_id("UP9000-NPUA80698_00-UNCHARTED2"));
    }

    #[test]
    fn plain_isos_stay_ps2() {
        assert_eq!(resolve_platform("God of War (USA)", "iso"), Some("ps2"));
        assert_eq!(resolve_platform("SLUS-20946 Some Game", "iso"), Some("ps2"));
        assert!(!contains_ps3_title_id("SLUS20946"));
    }

    #[test]
    fn psp_isos_detected_by_title_id() {
        assert_eq!(resolve_platform("Daxter [ULUS10041]", "iso"), Some("psp"));
        assert_eq!(
            resolve_platform("Ace Combat X - Skies of Deception [ULUS10314]", "iso"),
            Some("psp")
        );
        assert!(contains_psp_title_id("UCES00001 Ridge Racer"));
        // PS2 and PS3 IDs must not be misread as PSP.
        assert!(!contains_psp_title_id("SLUS20946"));
        assert!(!contains_psp_title_id("BLES02166"));
    }

    #[test]
    fn cso_defaults_psp_but_ps2_id_wins() {
        assert_eq!(resolve_platform("Some PSP Game", "cso"), Some("psp"));
        assert_eq!(resolve_platform("God of War [SLUS21243]", "cso"), Some("ps2"));
    }

    #[test]
    fn loose_pkg_is_not_a_game() {
        assert_eq!(resolve_platform("EP0002-BLES02166_00-UPDATE", "pkg"), None);
    }

    #[test]
    fn cd_extensions_default_to_ps1() {
        // CD formats that aren't `.iso` are PS1 by default, not PS2.
        assert_eq!(platform_for_extension("chd"), Some("ps1"));
        assert_eq!(platform_for_extension("cue"), Some("ps1"));
        assert_eq!(platform_for_extension("m3u"), Some("ps1"));
    }

    #[test]
    fn new_platform_extensions_resolve() {
        assert_eq!(platform_for_extension("gdi"), Some("dreamcast"));
        assert_eq!(platform_for_extension("pce"), Some("pcengine"));
        assert_eq!(platform_for_extension("a26"), Some("atari2600"));
        assert_eq!(platform_for_extension("ws"), Some("wonderswan"));
        assert_eq!(platform_for_extension("nsp"), Some("switch"));
        assert_eq!(platform_for_extension("3ds"), Some("n3ds"));
    }

    fn tmp(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn loose_bin_with_sega_header_is_genesis() {
        let mut bytes = vec![0u8; 0x104];
        bytes[0x100..0x104].copy_from_slice(b"SEGA");
        let (_d, path) = tmp("Sonic.bin", &bytes);
        let m = resolve_rom(&path, "Sonic", "bin").unwrap();
        assert_eq!(m.platform, "genesis");
    }

    #[test]
    fn loose_bin_without_signature_stays_ps2() {
        let (_d, path) = tmp("Mystery.bin", &vec![0u8; 0x200]);
        let m = resolve_rom(&path, "Mystery", "bin").unwrap();
        assert_eq!(m.platform, "ps2");
    }

    #[test]
    fn bin_track_paired_with_cue_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("Final Fantasy VII.bin");
        std::fs::write(&bin, vec![0u8; 16]).unwrap();
        std::fs::write(
            dir.path().join("Final Fantasy VII.cue"),
            "FILE \"Final Fantasy VII.bin\" BINARY\n",
        )
        .unwrap();
        // The .bin is a track; only the .cue is a game.
        assert!(resolve_rom(&bin, "Final Fantasy VII", "bin").is_none());
    }

    #[test]
    fn cue_sniffs_referenced_data_track() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = vec![0u8; 64];
        data.extend_from_slice(b"           PLAYSTATION  ");
        std::fs::write(dir.path().join("Game (Track 1).bin"), &data).unwrap();
        let cue = dir.path().join("Game.cue");
        std::fs::write(&cue, "FILE \"Game (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n").unwrap();
        let m = resolve_rom(&cue, "Game", "cue").unwrap();
        assert_eq!(m.platform, "ps1");
    }

    #[test]
    fn cue_defaults_to_ps1_when_unreadable() {
        // A cue pointing at a missing file still indexes as PS1, not nothing.
        let (_d, path) = tmp("Broken.cue", b"FILE \"missing.bin\" BINARY\n");
        let m = resolve_rom(&path, "Broken", "cue").unwrap();
        assert_eq!(m.platform, "ps1");
    }

    #[test]
    fn xbox_iso_is_detected() {
        let (_d, path) = tmp("Halo.iso", &crate::iso9660::testfixtures::xbox_iso());
        let m = resolve_rom(&path, "Halo", "iso").unwrap();
        assert_eq!(m.platform, "xbox");
    }

    #[test]
    fn zip_with_single_cartridge_routes_to_its_platform() {
        let zip = crate::archive::testfixtures::zip_with_names(&["Super Mario World.sfc"]);
        let (_d, path) = tmp("smw.zip", &zip);
        let m = resolve_rom(&path, "smw", "zip").unwrap();
        assert_eq!(m.platform, "snes");
    }

    #[test]
    fn zip_with_single_iso_routes_as_disc_not_arcade() {
        // A PS2 disc rip in a zip must not be mistaken for an arcade romset.
        let zip = crate::archive::testfixtures::zip_with_names(&["Big Mutha Truckers.iso"]);
        let (_d, path) = tmp("bmt.zip", &zip);
        assert_eq!(resolve_rom(&path, "bmt", "zip").unwrap().platform, "ps2");
    }

    #[test]
    fn zip_with_cue_and_bin_routes_to_ps1() {
        // The `.cue` is the disc marker; the `.bin` track must not drag it to
        // arcade. A PS1 game with split tracks still resolves to PS1.
        let zip = crate::archive::testfixtures::zip_with_names(&[
            "Blood Omen (Track 1).bin",
            "Blood Omen (Track 2).bin",
            "Blood Omen.cue",
        ]);
        let (_d, path) = tmp("blood.zip", &zip);
        assert_eq!(resolve_rom(&path, "blood", "zip").unwrap().platform, "ps1");
    }

    #[test]
    fn zip_with_psp_titled_iso_routes_to_psp() {
        // The inner filename's Sony title ID disambiguates the shared `.iso`.
        let zip = crate::archive::testfixtures::zip_with_names(&["Daxter [ULUS10041].iso"]);
        let (_d, path) = tmp("daxter.zip", &zip);
        assert_eq!(resolve_rom(&path, "daxter", "zip").unwrap().platform, "psp");
    }

    #[test]
    fn zip_with_gdi_routes_to_dreamcast() {
        // A Dreamcast GD-ROM rip (.gdi + raw tracks) is a disc, not arcade.
        let zip = crate::archive::testfixtures::zip_with_names(&[
            "Shenmue.gdi",
            "track01.bin",
            "track03.raw",
        ]);
        let (_d, path) = tmp("shenmue.zip", &zip);
        assert_eq!(resolve_rom(&path, "shenmue", "zip").unwrap().platform, "dreamcast");
    }

    #[test]
    fn zip_of_many_unrecognised_files_is_arcade() {
        let zip = crate::archive::testfixtures::zip_with_names(&["maincpu.u1", "gfx.u2", "snd.u3"]);
        let (_d, path) = tmp("sf2.zip", &zip);
        let m = resolve_rom(&path, "sf2", "zip").unwrap();
        assert_eq!(m.platform, "arcade");
    }

    #[test]
    fn unreadable_zip_falls_back_to_arcade() {
        let (_d, path) = tmp("broken.zip", b"not a real zip");
        let m = resolve_rom(&path, "broken", "zip").unwrap();
        assert_eq!(m.platform, "arcade");
    }

    #[test]
    fn unreadable_sevenzip_with_no_folder_hint_falls_back_to_arcade() {
        // A truncated 7z (signature only) can't be read; with nothing in the
        // path to hint a system, it defaults to arcade.
        let (_d, path) = tmp("neogeo.7z", b"7z\xBC\xAF\x27\x1C");
        let m = resolve_rom(&path, "neogeo", "7z").unwrap();
        assert_eq!(m.platform, "arcade");
    }

    /// Write a real `.7z` holding each name as a small stored entry, so the
    /// header reader (and routing) can be exercised end to end.
    fn write_7z(path: &std::path::Path, names: &[&str]) {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let mut w = ArchiveWriter::create(path).unwrap();
        w.set_encrypt_header(false);
        for name in names {
            let payload = vec![0u8; 64];
            w.push_archive_entry(ArchiveEntry::new_file(name), Some(&payload[..]))
                .unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn sevenzip_with_single_cartridge_routes_to_console() {
        // A 7z-compressed SNES ROM is identified by its inner `.sfc`, not arcade.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Mario Paint.7z");
        write_7z(&path, &["Mario Paint (Japan, USA) (En).sfc"]);
        assert_eq!(resolve_rom(&path, "Mario Paint", "7z").unwrap().platform, "snes");
    }

    #[test]
    fn sevenzip_disc_uses_folder_hint_to_disambiguate() {
        // A bare inner `.iso` is an ambiguous disc; the PSP system folder it
        // lives under decides it (instead of the `.iso` default, PS2).
        let dir = tempfile::tempdir().unwrap();
        let games = dir.path().join("Sony/PSP/Games");
        std::fs::create_dir_all(&games).unwrap();
        let path = games.join("Some PSP Game.7z");
        write_7z(&path, &["Some PSP Game.iso"]);
        assert_eq!(resolve_rom(&path, "Some PSP Game", "7z").unwrap().platform, "psp");
    }

    #[test]
    fn sevenzip_ps3_folder_dump_uses_folder_hint() {
        // A PS3 disc-folder dump in a 7z has no disc-image entry; the PS3 system
        // folder rescues it from the arcade default.
        let dir = tempfile::tempdir().unwrap();
        let games = dir.path().join("Sony/PS3/games");
        std::fs::create_dir_all(&games).unwrap();
        let path = games.join("Skate 2.7z");
        write_7z(
            &path,
            &[
                "Skate 2/PS3_DISC.SFB",
                "Skate 2/PS3_GAME/PARAM.SFO",
                "Skate 2/PS3_GAME/USRDIR/EBOOT.BIN",
            ],
        );
        assert_eq!(resolve_rom(&path, "Skate 2", "7z").unwrap().platform, "ps3");
    }

    #[test]
    fn folder_hint_reads_known_system_directories() {
        use std::path::Path;
        let hint = |p: &str| folder_platform_hint(Path::new(p));
        assert_eq!(hint("/roms/Sony/PS2/Games/ATV.7z"), Some("ps2"));
        assert_eq!(hint("/roms/Nintendo/SNES/Games/Mario.7z"), Some("snes"));
        assert_eq!(hint("/roms/Sony/PS3/games/Skate 2.7z"), Some("ps3"));
        assert_eq!(hint("/roms/Sony/PSX/Crash.cue"), Some("ps1"));
        // The deepest (most specific) matching component wins.
        assert_eq!(hint("/roms/PSP/PS2/God of War.iso"), Some("ps2"));
        // Combined and unrecognised folders give no hint — content decides.
        assert_eq!(hint("/roms/Nintendo/Wii_GC/Games/g.iso"), None);
        assert_eq!(hint("/some/random/place/game.iso"), None);
    }

    #[test]
    fn precise_content_is_not_overridden_by_folder() {
        // A specific cartridge extension is trusted even when the folder disagrees.
        let dir = tempfile::tempdir().unwrap();
        let mis = dir.path().join("Sony/PS2/Games");
        std::fs::create_dir_all(&mis).unwrap();
        let path = mis.join("Pocket Monsters.gba");
        std::fs::write(&path, vec![0u8; 16]).unwrap();
        assert_eq!(resolve_rom(&path, "Pocket Monsters", "gba").unwrap().platform, "gba");
    }
}
