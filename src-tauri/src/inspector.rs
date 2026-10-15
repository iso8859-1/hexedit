//! Decoding of basic data types at a position, without any template.

use encoding_rs::Encoding;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Entry {
    pub key: &'static str,
    pub label: &'static str,
    /// `None` when there are not enough bytes
    pub value: Option<String>,
    /// bytes covered by the value (for highlighting)
    pub size: u64,
    pub editable: bool,
}

/// Longest string decoded when nothing is selected.
const MAX_STR: usize = 256;

fn int_bytes<const N: usize>(d: &[u8], le: bool) -> Option<u128> {
    let b = d.get(..N)?;
    Some(if le {
        b.iter().rev().fold(0u128, |a, &x| (a << 8) | x as u128)
    } else {
        b.iter().fold(0u128, |a, &x| (a << 8) | x as u128)
    })
}

fn signed(raw: u128, bytes: u32) -> i128 {
    let s = 128 - bytes * 8;
    ((raw << s) as i128) >> s
}

/// `d` starts at the inspected offset; `sel_len` > 1 limits string decoding to the selection.
pub fn inspect(d: &[u8], sel_len: u64, le: bool, enc: &'static Encoding) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut push = |key, label, size: u64, editable, value: Option<String>| {
        out.push(Entry { key, label, value, size, editable });
    };

    push("bin", "Binary", 1, true, d.first().map(|b| format!("{b:08b}")));
    macro_rules! int {
        ($key_s:expr, $label_s:expr, $key_u:expr, $label_u:expr, $n:expr) => {{
            let raw = int_bytes::<$n>(d, le);
            push($key_s, $label_s, $n, true, raw.map(|r| signed(r, $n).to_string()));
            push($key_u, $label_u, $n, true, raw.map(|r| r.to_string()));
        }};
    }
    int!("i8", "Int8", "u8", "UInt8", 1);
    int!("i16", "Int16", "u16", "UInt16", 2);
    int!("i24", "Int24", "u24", "UInt24", 3);
    int!("i32", "Int32", "u32", "UInt32", 4);
    int!("i64", "Int64", "u64", "UInt64", 8);

    push("f16", "Half (float16)", 2, true, int_bytes::<2>(d, le).map(|r| fmt_float(f16_to_f32(r as u16))));
    push("f32", "Float (float32)", 4, true, int_bytes::<4>(d, le).map(|r| fmt_float(f32::from_bits(r as u32))));
    push("f64", "Double (float64)", 8, true, int_bytes::<8>(d, le).map(|r| fmt_float(f64::from_bits(r as u64))));

    let (uleb, ulen) = leb128(d, false);
    push("uleb128", "ULEB128", ulen, false, uleb);
    let (sleb, slen) = leb128(d, true);
    push("sleb128", "SLEB128", slen, false, sleb);

    // single characters
    let (cp_char, cp_len) = first_char(d, enc);
    push("char_cp", "Char (code page)", cp_len, false, cp_char);
    let (u8c, u8len) = first_utf8(d);
    push("char_utf8", "Char (UTF-8)", u8len, false, u8c);
    let (u16c, u16len) = first_utf16(d, le);
    push("char_utf16", "Char (UTF-16)", u16len, false, u16c);

    // strings: the selection, or up to the terminator
    let limit = if sel_len > 1 { (sel_len as usize).min(d.len()) } else { d.len().min(MAX_STR) };
    let s = &d[..limit];
    let cut = |len: usize| if sel_len > 1 { len } else { s.iter().position(|&b| b == 0).unwrap_or(len) };
    let n = cut(s.len());
    push("str_cp", "String (code page)", n as u64, false, Some(quote(&enc.decode_without_bom_handling(&s[..n]).0)));
    push("str_utf8", "String (UTF-8)", n as u64, false, Some(quote(&String::from_utf8_lossy(&s[..n]))));
    let units: Vec<u16> = s
        .chunks_exact(2)
        .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
        .collect();
    let un = if sel_len > 1 { units.len() } else { units.iter().position(|&u| u == 0).unwrap_or(units.len()) };
    push("str_utf16", "String (UTF-16)", un as u64 * 2, false, Some(quote(&String::from_utf16_lossy(&units[..un]))));

    // timestamps
    push("unix32", "Unix time (32 bit)", 4, false, int_bytes::<4>(d, le).map(|r| fmt_unix(signed(r, 4) as i64, 0)));
    push("unix64", "Unix time (64 bit)", 8, false, int_bytes::<8>(d, le).map(|r| fmt_unix(signed(r, 8) as i64, 0)));
    push(
        "filetime",
        "FILETIME",
        8,
        false,
        int_bytes::<8>(d, le).map(|r| {
            let ticks = r as u64;
            let secs = (ticks / 10_000_000) as i64 - 11_644_473_600;
            fmt_unix(secs, (ticks % 10_000_000) as u32 * 100)
        }),
    );
    push(
        "dos",
        "DOS date/time",
        4,
        false,
        int_bytes::<4>(d, le).map(|r| {
            let time = (r & 0xFFFF) as u32;
            let date = (r >> 16) as u32;
            let (y, mo, da) = (1980 + (date >> 9), (date >> 5) & 0xF, date & 0x1F);
            let (h, mi, s) = (time >> 11, (time >> 5) & 0x3F, (time & 0x1F) * 2);
            if mo == 0 || mo > 12 || da == 0 || h > 23 || mi > 59 || s > 59 {
                "invalid".to_string()
            } else {
                format!("{y:04}-{mo:02}-{da:02} {h:02}:{mi:02}:{s:02}")
            }
        }),
    );
    push(
        "oledate",
        "OLE date",
        8,
        false,
        int_bytes::<8>(d, le).map(|r| {
            let days = f64::from_bits(r as u64);
            if !days.is_finite() || days.abs() > 3_000_000.0 {
                return "out of range".into();
            }
            // OLE dates count days from 1899-12-30, which is unix day -25569
            let total_ms = ((days - 25569.0) * 86_400_000.0).round() as i64;
            fmt_unix(total_ms.div_euclid(1000), (total_ms.rem_euclid(1000) * 1_000_000) as u32)
        }),
    );
    push(
        "guid",
        "GUID",
        16,
        false,
        d.get(..16).map(|g| {
            // the first three groups follow the selected byte order (little endian on Windows)
            let (a, b, c) = if le {
                (
                    u32::from_le_bytes([g[0], g[1], g[2], g[3]]),
                    u16::from_le_bytes([g[4], g[5]]),
                    u16::from_le_bytes([g[6], g[7]]),
                )
            } else {
                (
                    u32::from_be_bytes([g[0], g[1], g[2], g[3]]),
                    u16::from_be_bytes([g[4], g[5]]),
                    u16::from_be_bytes([g[6], g[7]]),
                )
            };
            let rest: String = g[10..].iter().map(|b| format!("{b:02X}")).collect();
            format!("{{{a:08X}-{b:04X}-{c:04X}-{:02X}{:02X}-{rest}}}", g[8], g[9])
        }),
    );
    out
}

/// Encodes a user-entered value for an editable inspector entry.
pub fn encode(key: &str, text: &str, le: bool) -> Result<Vec<u8>, String> {
    let text = text.trim();
    let order = |mut v: Vec<u8>| {
        if !le {
            v.reverse();
        }
        v
    };
    let int = |bytes: usize, signed: bool| -> Result<Vec<u8>, String> {
        let v = parse_int(text)?;
        let bits = bytes as u32 * 8;
        let (min, max) = if signed {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        } else {
            (0, (1i128 << bits) - 1)
        };
        if v < min || v > max {
            return Err(format!("{v} is out of range {min}..={max}"));
        }
        Ok(order(v.to_le_bytes()[..bytes].to_vec()))
    };
    match key {
        "bin" => {
            let t = text.replace([' ', '_'], "");
            let t = t.strip_prefix("0b").unwrap_or(&t);
            if t.is_empty() || t.len() > 8 {
                return Err("enter up to 8 binary digits".into());
            }
            u8::from_str_radix(t, 2).map(|b| vec![b]).map_err(|_| "invalid binary number".into())
        }
        "i8" => int(1, true),
        "u8" => int(1, false),
        "i16" => int(2, true),
        "u16" => int(2, false),
        "i24" => int(3, true),
        "u24" => int(3, false),
        "i32" => int(4, true),
        "u32" => int(4, false),
        "i64" => int(8, true),
        "u64" => int(8, false),
        "f16" => {
            let f: f32 = text.parse().map_err(|_| "invalid number")?;
            Ok(order(f32_to_f16(f).to_le_bytes().to_vec()))
        }
        "f32" => {
            let f: f32 = text.parse().map_err(|_| "invalid number")?;
            Ok(order(f.to_le_bytes().to_vec()))
        }
        "f64" => {
            let f: f64 = text.parse().map_err(|_| "invalid number")?;
            Ok(order(f.to_le_bytes().to_vec()))
        }
        _ => Err(format!("'{key}' cannot be edited")),
    }
}

fn parse_int(t: &str) -> Result<i128, String> {
    let t = t.replace('_', "");
    let (neg, body) = match t.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, t.clone()),
    };
    let lower = body.to_ascii_lowercase();
    let v = if let Some(h) = lower.strip_prefix("0x") {
        i128::from_str_radix(h, 16)
    } else if let Some(b) = lower.strip_prefix("0b") {
        i128::from_str_radix(b, 2)
    } else {
        lower.parse::<i128>()
    }
    .map_err(|_| format!("invalid integer '{t}'"))?;
    Ok(if neg { -v } else { v })
}

fn leb128(d: &[u8], signed: bool) -> (Option<String>, u64) {
    let mut result: i128 = 0;
    let mut shift = 0u32;
    for (i, &b) in d.iter().enumerate().take(10) {
        result |= ((b & 0x7F) as i128) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if signed && b & 0x40 != 0 {
                result -= 1i128 << shift;
            }
            return (Some(result.to_string()), i as u64 + 1);
        }
    }
    (None, 0)
}

fn first_char(d: &[u8], enc: &'static Encoding) -> (Option<String>, u64) {
    // try increasingly long prefixes until one decodes to a single character
    for len in 1..=d.len().min(4) {
        let (s, had_errors) = enc.decode_without_bom_handling(&d[..len]);
        if !had_errors && s.chars().count() == 1 {
            return (Some(describe_char(s.chars().next().unwrap())), len as u64);
        }
    }
    if d.is_empty() {
        (None, 1)
    } else {
        (Some("invalid".into()), 1)
    }
}

fn first_utf8(d: &[u8]) -> (Option<String>, u64) {
    let Some(&b0) = d.first() else { return (None, 1) };
    let len = match b0 {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return (Some("invalid".into()), 1),
    };
    match d.get(..len).and_then(|b| std::str::from_utf8(b).ok()) {
        Some(s) => (Some(describe_char(s.chars().next().unwrap())), len as u64),
        None => (Some("invalid".into()), 1),
    }
}

fn first_utf16(d: &[u8], le: bool) -> (Option<String>, u64) {
    let unit = |i: usize| {
        d.get(i..i + 2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
    };
    let Some(u0) = unit(0) else { return (None, 2) };
    let units: Vec<u16> = match (u0, unit(2)) {
        (0xD800..=0xDBFF, Some(u1 @ 0xDC00..=0xDFFF)) => vec![u0, u1],
        _ => vec![u0],
    };
    let len = units.len() as u64 * 2;
    match char::decode_utf16(units).next() {
        Some(Ok(c)) => (Some(describe_char(c)), len),
        _ => (Some("invalid".into()), len),
    }
}

fn describe_char(c: char) -> String {
    let code = c as u32;
    if c.is_control() {
        format!("U+{code:04X} (control)")
    } else {
        format!("'{c}' U+{code:04X}")
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Shortest round-trip text, in scientific notation for very small or large values.
pub fn fmt_float<T: std::fmt::Display + std::fmt::LowerExp + Into<f64> + Copy>(v: T) -> String {
    let a = v.into().abs();
    if a.is_finite() && a != 0.0 && !(1e-5..1e15).contains(&a) {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1F) as i32;
    let frac = (h & 0x3FF) as f32;
    sign * match exp {
        0 => frac * 2f32.powi(-24),
        31 => {
            if frac == 0.0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

pub fn f32_to_f16(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let man = bits & 0x7F_FFFF;
    if exp == 0xFF {
        return sign | 0x7C00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // subnormal: shift the implicit 1 in, round to nearest even
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rem = m & ((1 << shift) - 1);
        let mut r = m >> shift;
        if rem > half || (rem == half && r & 1 == 1) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1FFF;
    if rem > 0x1000 || (rem == 0x1000 && r & 1 == 1) {
        r += 1; // may carry into the exponent, which is the correct rounding
    }
    sign | r as u16
}

/// Formats seconds (+ nanoseconds) since the unix epoch as UTC.
fn fmt_unix(secs: i64, nanos: u32) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    if !(-9999..=9999).contains(&y) {
        return "out of range".into();
    }
    let base = format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", tod / 3600, tod % 3600 / 60, tod % 60);
    if nanos == 0 {
        format!("{base} UTC")
    } else {
        let frac = format!("{nanos:09}");
        format!("{base}.{} UTC", frac.trim_end_matches('0'))
    }
}

/// Howard Hinnant's days -> civil date algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(e: &'a [Entry], k: &str) -> &'a str {
        e.iter().find(|x| x.key == k).unwrap().value.as_deref().unwrap_or("-")
    }

    #[test]
    fn integers_and_floats() {
        let d = [0xFE, 0xFF, 0xFF, 0xFF, 0, 0, 0x80, 0x3F];
        let e = inspect(&d, 0, true, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "i8"), "-2");
        assert_eq!(get(&e, "u8"), "254");
        assert_eq!(get(&e, "i32"), "-2");
        assert_eq!(get(&e, "u16"), "65534");
        assert_eq!(get(&e, "bin"), "11111110");
        let f = [0, 0, 0x80, 0x3F];
        let e = inspect(&f, 0, true, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "f32"), "1");
        assert_eq!(get(&e, "i64"), "-");
        let e = inspect(&[0x3F, 0x80, 0, 0], 0, false, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "f32"), "1");
    }

    #[test]
    fn strings_and_chars() {
        let d = b"Gr\xfc\xdf\0rest";
        let e = inspect(d, 0, true, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "str_cp"), "\"Grüß\"");
        let e = inspect("€x".as_bytes(), 0, true, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "char_utf8"), "'€' U+20AC");
        let u16 = [0x3D, 0xD8, 0x00, 0xDE];
        let e = inspect(&u16, 0, true, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "char_utf16"), "'😀' U+1F600");
    }

    #[test]
    fn times() {
        assert_eq!(fmt_unix(0, 0), "1970-01-01 00:00:00 UTC");
        assert_eq!(fmt_unix(951_782_400, 0), "2000-02-29 00:00:00 UTC");
        // 2020-01-01 as FILETIME
        let ft: u64 = (1_577_836_800 + 11_644_473_600) * 10_000_000;
        let e = inspect(&ft.to_le_bytes(), 0, true, encoding_rs::WINDOWS_1252);
        assert_eq!(get(&e, "filetime"), "2020-01-01 00:00:00 UTC");
    }

    #[test]
    fn encode_values() {
        assert_eq!(encode("u16", "0x1234", true).unwrap(), vec![0x34, 0x12]);
        assert_eq!(encode("u16", "0x1234", false).unwrap(), vec![0x12, 0x34]);
        assert_eq!(encode("i8", "-1", true).unwrap(), vec![0xFF]);
        assert!(encode("u8", "256", true).is_err());
        assert!(encode("i8", "-129", true).is_err());
        assert_eq!(encode("f32", "1", true).unwrap(), vec![0, 0, 0x80, 0x3F]);
        assert_eq!(encode("bin", "1010", true).unwrap(), vec![10]);
        for v in [0.0f32, 1.0, -2.5, 65504.0, 2f32.powi(-14), 2f32.powi(-24), 3.0 * 2f32.powi(-20)] {
            assert_eq!(f16_to_f32(f32_to_f16(v)), v, "{v}");
        }
    }

    #[test]
    fn leb() {
        assert_eq!(leb128(&[0xE5, 0x8E, 0x26], false), (Some("624485".into()), 3));
        assert_eq!(leb128(&[0xC0, 0xBB, 0x78], true), (Some("-123456".into()), 3));
    }
}
