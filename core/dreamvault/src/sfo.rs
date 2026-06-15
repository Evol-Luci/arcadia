//! Minimal PARAM.SFO (PSF) reader.
//!
//! PS3 disc dumps and PKG installs carry a `PARAM.SFO` whose `TITLE` field is
//! the real human game name (e.g. "Call of Duty®: Black Ops III"), which is far
//! better than guessing from a folder name like `BLES02166-[...]`. We only need
//! to read string fields, so this is a small, dependency-free parser rather than
//! a full PSF implementation.
//!
//! Layout (all integers little-endian):
//!   header (20 bytes): magic "\0PSF", version u32, key_table_start u32,
//!                      data_table_start u32, entry_count u32
//!   index table: entry_count × 16 bytes
//!     key_offset u16   (relative to key_table_start)
//!     data_fmt   u16   (0x0204 / 0x0004 = UTF-8 string, 0x0404 = u32)
//!     data_len   u32   (used bytes, incl. trailing NUL for strings)
//!     data_max   u32   (reserved capacity, ignored)
//!     data_offset u32  (relative to data_table_start)

use std::path::Path;

/// Read the `TITLE` field from a PS3 `PARAM.SFO`. Returns `None` on any I/O or
/// parse failure, or if there is no usable title.
pub fn read_title(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    read_field(&bytes, "TITLE")
}

/// Read a string-typed field (e.g. "TITLE", "TITLE_ID") from SFO bytes.
pub fn read_field(bytes: &[u8], key: &str) -> Option<String> {
    if bytes.len() < 20 || &bytes[0..4] != b"\x00PSF" {
        return None;
    }
    let key_table_start = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    let data_table_start = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let entry_count = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;

    const INDEX_START: usize = 20;
    for i in 0..entry_count {
        let e = INDEX_START + i * 16;
        if e + 16 > bytes.len() {
            break;
        }
        let key_offset = u16::from_le_bytes(bytes[e..e + 2].try_into().ok()?) as usize;
        let data_fmt = u16::from_le_bytes(bytes[e + 2..e + 4].try_into().ok()?);
        let data_len = u32::from_le_bytes(bytes[e + 4..e + 8].try_into().ok()?) as usize;
        let data_offset = u32::from_le_bytes(bytes[e + 12..e + 16].try_into().ok()?) as usize;

        if read_cstr(bytes, key_table_start.checked_add(key_offset)?)? != key {
            continue;
        }
        // Only string formats carry a name.
        if data_fmt != 0x0204 && data_fmt != 0x0004 {
            return None;
        }
        let start = data_table_start.checked_add(data_offset)?;
        let end = start.checked_add(data_len)?;
        let raw = bytes.get(start..end)?;
        let trimmed = &raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())];
        let s = String::from_utf8_lossy(trimmed).trim().to_string();
        return (!s.is_empty()).then_some(s);
    }
    None
}

/// Read a NUL-terminated string starting at `start`.
fn read_cstr(bytes: &[u8], start: usize) -> Option<String> {
    let rest = bytes.get(start..)?;
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid SFO with the given string key/value pairs.
    fn make_sfo(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut keys = Vec::new();
        let mut key_offsets = Vec::new();
        for (k, _) in pairs {
            key_offsets.push(keys.len() as u16);
            keys.extend_from_slice(k.as_bytes());
            keys.push(0);
        }
        let mut data = Vec::new();
        let mut data_offsets = Vec::new();
        let mut data_lens = Vec::new();
        for (_, v) in pairs {
            data_offsets.push(data.len() as u32);
            let mut bytes = v.as_bytes().to_vec();
            bytes.push(0);
            data_lens.push(bytes.len() as u32);
            data.extend_from_slice(&bytes);
        }

        let key_table_start = 20 + pairs.len() * 16;
        let data_table_start = key_table_start + keys.len();

        let mut out = Vec::new();
        out.extend_from_slice(b"\x00PSF");
        out.extend_from_slice(&0x0101_0000u32.to_le_bytes());
        out.extend_from_slice(&(key_table_start as u32).to_le_bytes());
        out.extend_from_slice(&(data_table_start as u32).to_le_bytes());
        out.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
        for i in 0..pairs.len() {
            out.extend_from_slice(&key_offsets[i].to_le_bytes());
            out.extend_from_slice(&0x0204u16.to_le_bytes());
            out.extend_from_slice(&data_lens[i].to_le_bytes());
            out.extend_from_slice(&data_lens[i].to_le_bytes());
            out.extend_from_slice(&data_offsets[i].to_le_bytes());
        }
        out.extend_from_slice(&keys);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn reads_title_and_title_id() {
        let sfo = make_sfo(&[
            ("APP_VER", "01.00"),
            ("TITLE", "Call of Duty: Black Ops III"),
            ("TITLE_ID", "BLES02166"),
        ]);
        assert_eq!(
            read_field(&sfo, "TITLE").as_deref(),
            Some("Call of Duty: Black Ops III")
        );
        assert_eq!(read_field(&sfo, "TITLE_ID").as_deref(), Some("BLES02166"));
        assert_eq!(read_field(&sfo, "MISSING"), None);
    }

    #[test]
    fn rejects_non_sfo() {
        assert_eq!(read_field(b"not an sfo at all", "TITLE"), None);
        assert_eq!(read_field(&[], "TITLE"), None);
    }
}
