//! LaunchBox Games Database cover provider. Opt-in, fallback-after-libretro.
//!
//! LaunchBox has no API; the sanctioned path (endorsed by LaunchBox's founder)
//! is the daily `Metadata.zip` dump. We stream-parse it into a compact sidecar
//! SQLite index keyed by (arcadia-platform, normalized-name) -> image file
//! path, then resolve covers offline. Cover images only; text stays
//! ScreenScraper's job.

/// Sanctioned metadata dump (the whole catalogue; hundreds of MB).
pub const METADATA_URL: &str = "https://gamesdb.launchbox-app.com/Metadata.zip";
/// Base for image URLs built from a GameImage `FileName`.
pub const IMAGE_BASE: &str = "https://images.launchbox-app.com";

/// Lowercase, replace every non-alphanumeric run with a single space, trim.
/// Used as the join key so "Pokémon: Red!" and "pokemon red" collide.
pub(crate) fn normalize_name(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a LaunchBox `<Platform>` name to an Arcadia platform slug. `None` for
/// platforms Arcadia doesn't track (those games are skipped at index time).
pub(crate) fn arcadia_slug_for_launchbox(name: &str) -> Option<&'static str> {
    Some(match name {
        "Nintendo Entertainment System" => "nes",
        "Super Nintendo Entertainment System" => "snes",
        "Nintendo 64" => "n64",
        "Nintendo GameCube" => "gamecube",
        "Nintendo Wii" => "wii",
        "Nintendo Game Boy" => "gb",
        "Nintendo Game Boy Color" => "gbc",
        "Nintendo Game Boy Advance" => "gba",
        "Nintendo DS" => "nds",
        "Nintendo Virtual Boy" => "virtualboy",
        "Sega Genesis" => "genesis",
        "Sega Dreamcast" => "dreamcast",
        "Sony Playstation" => "ps1",
        "Sony Playstation 2" => "ps2",
        "Sony PSP" => "psp",
        "Sony Playstation 3" => "ps3",
        "Sega Saturn" => "saturn",
        "Sega CD" => "segacd",
        "Sega 32X" => "sega32x",
        "Sega Game Gear" => "gamegear",
        "Sega Master System" => "sms",
        "NEC TurboGrafx-16" => "pcengine",
        "Nintendo 3DS" => "n3ds",
        "Atari 2600" => "atari2600",
        "Atari 5200" => "atari5200",
        "Atari 7800" => "atari7800",
        "Atari Jaguar" => "jaguar",
        "Atari Lynx" => "lynx",
        "Commodore Amiga" => "amiga",
        "Commodore 64" => "c64",
        "ColecoVision" => "colecovision",
        "Microsoft MSX" => "msx",
        "Microsoft Xbox" => "xbox",
        "SNK Neo Geo Pocket" => "ngp",
        "WonderSwan" => "wonderswan",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_punctuation_and_case() {
        assert_eq!(normalize_name("Pokémon: Red!"), "pok mon red");
        assert_eq!(normalize_name("  Super   Mario  64 "), "super mario 64");
        assert_eq!(normalize_name("The Legend of Zelda"), "the legend of zelda");
    }

    #[test]
    fn platform_mapping_known_and_unknown() {
        assert_eq!(arcadia_slug_for_launchbox("Super Nintendo Entertainment System"), Some("snes"));
        assert_eq!(arcadia_slug_for_launchbox("Sony Playstation"), Some("ps1"));
        assert_eq!(arcadia_slug_for_launchbox("Sega Pico"), None);
    }
}
