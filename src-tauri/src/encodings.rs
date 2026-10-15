//! Code page handling for text display, string decoding and search.

use encoding_rs::Encoding;
use serde::Serialize;

/// Encodings offered in the UI (encoding_rs names).
const NAMES: &[&str] = &[
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "windows-1253",
    "windows-1254",
    "windows-1255",
    "windows-1256",
    "windows-1257",
    "windows-1258",
    "windows-874",
    "ISO-8859-2",
    "ISO-8859-3",
    "ISO-8859-4",
    "ISO-8859-5",
    "ISO-8859-6",
    "ISO-8859-7",
    "ISO-8859-8",
    "ISO-8859-10",
    "ISO-8859-13",
    "ISO-8859-14",
    "ISO-8859-15",
    "ISO-8859-16",
    "KOI8-R",
    "KOI8-U",
    "IBM866",
    "macintosh",
    "x-mac-cyrillic",
    "Shift_JIS",
    "EUC-JP",
    "GBK",
    "gb18030",
    "Big5",
    "EUC-KR",
    "UTF-8",
];

#[derive(Serialize)]
pub struct EncodingList {
    /// the system's ANSI code page
    pub local: String,
    pub local_cp: u32,
    pub list: Vec<String>,
}

#[cfg(windows)]
fn local_codepage() -> u32 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetACP() -> u32;
    }
    unsafe { GetACP() }
}

#[cfg(not(windows))]
fn local_codepage() -> u32 {
    65001
}

pub fn local() -> &'static Encoding {
    codepage::to_encoding(local_codepage() as u16).unwrap_or(encoding_rs::WINDOWS_1252)
}

pub fn list() -> EncodingList {
    EncodingList {
        local: local().name().to_string(),
        local_cp: local_codepage(),
        list: NAMES.iter().map(|s| s.to_string()).collect(),
    }
}

/// Resolves an encoding name; "local" or unknown names give the system code page.
pub fn lookup(name: &str) -> &'static Encoding {
    if name == "local" {
        return local();
    }
    Encoding::for_label(name.as_bytes()).unwrap_or_else(local)
}

/// Display character for each byte value; empty where the byte is not a
/// printable character on its own (control codes, multi-byte lead bytes).
pub fn char_table(enc: &'static Encoding) -> Vec<String> {
    (0..=255u8)
        .map(|b| {
            let byte = [b];
            let (s, had_errors) = enc.decode_without_bom_handling(&byte);
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !had_errors && !c.is_control() && c != '\u{FFFD}' => c.to_string(),
                _ => String::new(),
            }
        })
        .collect()
}

/// Converts a search query into the byte pattern to look for.
pub fn search_pattern(query: &str, mode: &str, encoding: &str, little_endian: bool) -> Result<Vec<u8>, String> {
    match mode {
        "hex" => {
            let digits: String = query.chars().filter(|c| !c.is_whitespace()).collect();
            let digits = digits.replace("0x", "").replace(',', "");
            if !digits.len().is_multiple_of(2) {
                return Err("hex search needs an even number of digits".into());
            }
            (0..digits.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| format!("invalid hex '{}'", &digits[i..i + 2])))
                .collect()
        }
        "text" => {
            let enc = lookup(encoding);
            let (bytes, _, unmappable) = enc.encode(query);
            if unmappable {
                return Err(format!("the text cannot be represented in {}", enc.name()));
            }
            Ok(bytes.into_owned())
        }
        "utf8" => Ok(query.as_bytes().to_vec()),
        "utf16" => Ok(query
            .encode_utf16()
            .flat_map(|u| if little_endian { u.to_le_bytes() } else { u.to_be_bytes() })
            .collect()),
        _ => Err(format!("unknown search mode '{mode}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_and_patterns() {
        let t = char_table(encoding_rs::WINDOWS_1252);
        assert_eq!(t[b'A' as usize], "A");
        assert_eq!(t[0x80], "€");
        assert_eq!(t[0x0A], "");
        assert_eq!(search_pattern("de ad BE ef", "hex", "", true).unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(search_pattern("abc", "hex", "", true).is_err());
        assert_eq!(search_pattern("ü", "text", "windows-1252", true).unwrap(), vec![0xFC]);
        assert_eq!(search_pattern("A", "utf16", "", false).unwrap(), vec![0, 0x41]);
    }
}
