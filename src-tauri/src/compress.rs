//! Decompressing a byte range (deflate, zlib, gzip) into a new document.

use std::io::Read;

use flate2::bufread::{DeflateDecoder, GzDecoder, ZlibDecoder};
use serde::Serialize;

/// Largest decompressed size accepted, so a decompression bomb cannot take all memory.
pub const MAX_OUTPUT: usize = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Deflate,
    Zlib,
    Gzip,
}

impl Format {
    pub fn parse(s: &str) -> Result<Option<Format>, String> {
        Ok(match s {
            "auto" => None,
            "deflate" => Some(Format::Deflate),
            "zlib" => Some(Format::Zlib),
            "gzip" => Some(Format::Gzip),
            _ => return Err(format!("unknown compression format '{s}'")),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Deflate => "deflate",
            Format::Zlib => "zlib",
            Format::Gzip => "gzip",
        }
    }

    /// The format whose header `data` starts with; raw deflate has none.
    pub fn detect(data: &[u8]) -> Format {
        match data {
            [0x1F, 0x8B, 8, ..] => Format::Gzip,
            // CM = 8 (deflate), window size <= 32K, header checksum
            [cmf, flg, ..] if cmf & 0x0F == 8 && cmf >> 4 <= 7 && (u16::from(*cmf) << 8 | u16::from(*flg)) % 31 == 0 => {
                Format::Zlib
            }
            _ => Format::Deflate,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inflated {
    #[serde(skip)]
    pub data: Vec<u8>,
    pub format: &'static str,
    /// input bytes the compressed stream used; the rest of the range is trailing data
    pub consumed: usize,
    /// None when the stream ended properly, else why decoding stopped
    /// (`data` then holds what was decoded up to that point)
    pub error: Option<String>,
}

/// Decompresses `input` as `format`, or the format its header suggests.
/// Fails only when nothing at all could be decoded.
pub fn inflate(input: &[u8], format: Option<Format>, limit: usize) -> Result<Inflated, String> {
    let detected = Format::detect(input);
    let first = run(input, format.unwrap_or(detected), limit)?;
    // a guessed header can be a coincidence: then the data may still be raw deflate
    if format.is_none() && detected != Format::Deflate && (first.error.is_some() || first.data.is_empty()) {
        if let Ok(raw) = run(input, Format::Deflate, limit) {
            if raw.error.is_none() && !raw.data.is_empty() {
                return Ok(raw);
            }
        }
    }
    if first.data.is_empty() {
        if let Some(e) = first.error {
            return Err(format!("not valid {} data: {e}", first.format));
        }
    }
    Ok(first)
}

fn run(input: &[u8], format: Format, limit: usize) -> Result<Inflated, String> {
    let mut rest = input;
    let mut data = Vec::new();
    // bufread decoders consume exactly the bytes of the stream, so `rest` ends
    // up holding the trailing data
    let result = {
        let reader: Box<dyn Read + '_> = match format {
            Format::Deflate => Box::new(DeflateDecoder::new(&mut rest)),
            Format::Zlib => Box::new(ZlibDecoder::new(&mut rest)),
            Format::Gzip => Box::new(GzDecoder::new(&mut rest)),
        };
        // read_to_end keeps what was decoded before an error
        reader.take(limit as u64 + 1).read_to_end(&mut data)
    };
    if data.len() > limit {
        return Err(format!("the decompressed data is larger than {} MiB", limit >> 20));
    }
    let error = match result {
        Ok(_) if rest.len() == input.len() => Some("no compressed data".into()),
        Ok(_) => None,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Some("the compressed data ends early".into()),
        Err(e) => Some(e.to_string()),
    };
    Ok(Inflated { data, format: format.name(), consumed: input.len() - rest.len(), error })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    fn text() -> Vec<u8> {
        (0..2000).flat_map(|i| format!("line {i}\n").into_bytes()).collect()
    }

    fn deflate(d: &[u8]) -> Vec<u8> {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(d).unwrap();
        e.finish().unwrap()
    }

    fn zlib(d: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(d).unwrap();
        e.finish().unwrap()
    }

    fn gzip(d: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(d).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn detects_and_decodes_each_format() {
        let t = text();
        for (packed, name) in [(deflate(&t), "deflate"), (zlib(&t), "zlib"), (gzip(&t), "gzip")] {
            let r = inflate(&packed, None, MAX_OUTPUT).unwrap();
            assert_eq!((r.format, r.consumed, r.error.as_deref()), (name, packed.len(), None));
            assert_eq!(r.data, t);
            // an explicit format gives the same result
            let r = inflate(&packed, Format::parse(name).unwrap(), MAX_OUTPUT).unwrap();
            assert_eq!(r.data, t);
        }
        assert!(Format::parse("lzma").is_err());
    }

    #[test]
    fn trailing_and_truncated_data() {
        let t = text();
        let mut packed = zlib(&t);
        let len = packed.len();
        packed.extend_from_slice(b"TRAILER");
        let r = inflate(&packed, None, MAX_OUTPUT).unwrap();
        assert_eq!((r.consumed, r.error.as_deref(), r.data.len()), (len, None, t.len()));

        let r = inflate(&packed[..len / 2], None, MAX_OUTPUT).unwrap();
        assert_eq!(r.error.as_deref(), Some("the compressed data ends early"));
        assert!(!r.data.is_empty() && t.starts_with(&r.data));
    }

    #[test]
    fn rejects_garbage_and_bombs() {
        let e = inflate(&[0xFF; 64], None, MAX_OUTPUT).unwrap_err();
        assert!(e.starts_with("not valid deflate data"), "{e}");
        assert!(inflate(&[], None, MAX_OUTPUT).is_err());
        // 1 MiB of zeros compresses to about 1 KiB
        let packed = deflate(&vec![0; 1 << 20]);
        let e = inflate(&packed, None, 1 << 19).unwrap_err();
        assert!(e.contains("larger than"), "{e}");
    }

    #[test]
    fn raw_deflate_that_looks_like_zlib() {
        // a non-final stored block of 1 byte, then an empty final one: its first
        // two bytes 78 01 are also a valid zlib header
        let raw = [0x78, 0x01, 0x00, 0xFE, 0xFF, b'A', 0x01, 0x00, 0x00, 0xFF, 0xFF];
        assert_eq!(Format::detect(&raw), Format::Zlib);
        let r = inflate(&raw, None, MAX_OUTPUT).unwrap();
        assert_eq!((r.format, r.data.as_slice(), r.consumed), ("deflate", &b"A"[..], raw.len()));
        // asked for zlib explicitly, there is no fallback
        assert!(inflate(&raw, Some(Format::Zlib), MAX_OUTPUT).is_err());
    }
}
