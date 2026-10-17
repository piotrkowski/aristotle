//! Tiny text decoders for the legacy encodings books actually use.
//!
//! A general-purpose crate such as `encoding_rs` would add a few hundred
//! kilobytes of tables to the device binary for encodings we never meet. The
//! owner's library is mostly Polish, so the encodings that matter are UTF-8,
//! UTF-16 (PDF strings, some OPFs), Windows-1250 and ISO-8859-2 (old Polish
//! files), Windows-1252 (MOBI files from Western tools) and PDFDocEncoding.
//! Each single-byte table only lists the upper half; the lower half is ASCII.

/// Encodings we can decode. Unknown labels fall back to heuristics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1250,
    Windows1252,
    Iso8859_1,
    Iso8859_2,
    PdfDoc,
}

impl Encoding {
    /// Maps a charset label (XML declaration, HTTP-style name) to an encoding.
    pub fn from_label(label: &str) -> Option<Encoding> {
        let l = label.trim().to_ascii_lowercase().replace('_', "-");
        Some(match l.as_str() {
            "utf-8" | "utf8" => Encoding::Utf8,
            "utf-16" | "utf-16le" => Encoding::Utf16Le,
            "utf-16be" => Encoding::Utf16Be,
            "windows-1250" | "cp1250" | "x-cp1250" => Encoding::Windows1250,
            "windows-1252" | "cp1252" | "x-cp1252" => Encoding::Windows1252,
            "iso-8859-1" | "latin1" | "latin-1" | "us-ascii" | "ascii" => Encoding::Iso8859_1,
            "iso-8859-2" | "latin2" | "latin-2" => Encoding::Iso8859_2,
            _ => return None,
        })
    }
}

/// Decodes `bytes` with `enc`. Invalid sequences become U+FFFD; nothing panics.
pub fn decode(bytes: &[u8], enc: Encoding) -> String {
    match enc {
        Encoding::Utf8 => {
            let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
            String::from_utf8_lossy(b).into_owned()
        }
        Encoding::Utf16Le => decode_utf16(bytes, false),
        Encoding::Utf16Be => decode_utf16(bytes, true),
        Encoding::Windows1250 => decode_single(bytes, &CP1250),
        Encoding::Windows1252 => decode_single(bytes, &CP1252),
        Encoding::Iso8859_1 => bytes.iter().map(|&b| b as char).collect(),
        Encoding::Iso8859_2 => decode_single(bytes, &ISO8859_2),
        Encoding::PdfDoc => bytes
            .iter()
            .map(|&b| match b {
                0x18..=0x1F => {
                    char::from_u32(PDFDOC_LOW[(b - 0x18) as usize] as u32).unwrap_or('\u{FFFD}')
                }
                0x80..=0xA0 => {
                    char::from_u32(PDFDOC_HIGH[(b - 0x80) as usize] as u32).unwrap_or('\u{FFFD}')
                }
                _ => b as char,
            })
            .collect(),
    }
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> String {
    let mut b = bytes;
    // Honour a BOM even if it contradicts the label; the BOM is the stronger signal.
    let mut be = big_endian;
    if b.starts_with(&[0xFE, 0xFF]) {
        be = true;
        b = &b[2..];
    } else if b.starts_with(&[0xFF, 0xFE]) {
        be = false;
        b = &b[2..];
    }
    let units = b.as_chunks::<2>().0.iter().map(|c| {
        if be {
            u16::from_be_bytes([c[0], c[1]])
        } else {
            u16::from_le_bytes([c[0], c[1]])
        }
    });
    char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect()
}

fn decode_single(bytes: &[u8], upper: &[u16; 128]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                char::from_u32(upper[(b - 0x80) as usize] as u32).unwrap_or('\u{FFFD}')
            }
        })
        .collect()
}

/// Decodes bytes of unknown encoding: BOMs first, then UTF-8 if valid, else
/// a single-byte guess.
///
/// The guess prefers Windows-1250 when the bytes contain letters that only
/// make sense as Polish in that code page (ą ś ź ł ż and capitals), because a
/// misread as Windows-1252 turns them into ¹ œ Ÿ ³ ¿, which no real title
/// contains. Otherwise Windows-1252, the usual default of Western tools.
pub fn decode_guess(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return decode(bytes, Encoding::Utf8);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode(bytes, Encoding::Utf16Be);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode(bytes, Encoding::Utf16Le);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => decode(bytes, guess_single_byte(bytes)),
    }
}

fn guess_single_byte(bytes: &[u8]) -> Encoding {
    // ą ś ź ł ż Ą Ś Ź Ł Ż in Windows-1250.
    const POLISH_1250: [u8; 10] = [0xB9, 0x9C, 0x9F, 0xB3, 0xBF, 0xA5, 0x8C, 0x8F, 0xA3, 0xAF];
    if bytes.iter().any(|b| POLISH_1250.contains(b)) {
        Encoding::Windows1250
    } else {
        Encoding::Windows1252
    }
}

/// Decodes an XML document: BOM, then the `encoding` pseudo-attribute of the
/// XML declaration, then [`decode_guess`]. The result is always a `String`
/// that `roxmltree` can parse (it ignores the declared encoding itself).
pub fn decode_xml(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFE, 0xFF])
        || bytes.starts_with(&[0xFF, 0xFE])
    {
        return decode_guess(bytes);
    }
    // UTF-16 without a BOM: "<\0?\0" or "\0<\0?".
    if bytes.starts_with(b"<\0?\0") {
        return decode(bytes, Encoding::Utf16Le);
    }
    if bytes.starts_with(b"\0<\0?") {
        return decode(bytes, Encoding::Utf16Be);
    }
    if let Some(label) = xml_declared_encoding(bytes) {
        if let Some(enc) = Encoding::from_label(&label) {
            if enc == Encoding::Utf8 {
                // Declared UTF-8 but broken: still try the heuristics.
                return decode_guess(bytes);
            }
            if enc != Encoding::Utf16Le && enc != Encoding::Utf16Be {
                return decode(bytes, enc);
            }
        }
    }
    decode_guess(bytes)
}

fn xml_declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(256)];
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(&head[..end]).ok()?;
    let at = decl.find("encoding")?;
    let rest = decl[at + 8..].trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    Some(rest[..rest.find(quote)?].to_owned())
}

// Undefined positions map to the same code point, as WHATWG does, so that
// nothing is silently dropped.
#[rustfmt::skip]
const CP1252: [u16; 128] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F,
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
    0x00A0, 0x00A1, 0x00A2, 0x00A3, 0x00A4, 0x00A5, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x00AA, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x00AF,
    0x00B0, 0x00B1, 0x00B2, 0x00B3, 0x00B4, 0x00B5, 0x00B6, 0x00B7, 0x00B8, 0x00B9, 0x00BA, 0x00BB, 0x00BC, 0x00BD, 0x00BE, 0x00BF,
    0x00C0, 0x00C1, 0x00C2, 0x00C3, 0x00C4, 0x00C5, 0x00C6, 0x00C7, 0x00C8, 0x00C9, 0x00CA, 0x00CB, 0x00CC, 0x00CD, 0x00CE, 0x00CF,
    0x00D0, 0x00D1, 0x00D2, 0x00D3, 0x00D4, 0x00D5, 0x00D6, 0x00D7, 0x00D8, 0x00D9, 0x00DA, 0x00DB, 0x00DC, 0x00DD, 0x00DE, 0x00DF,
    0x00E0, 0x00E1, 0x00E2, 0x00E3, 0x00E4, 0x00E5, 0x00E6, 0x00E7, 0x00E8, 0x00E9, 0x00EA, 0x00EB, 0x00EC, 0x00ED, 0x00EE, 0x00EF,
    0x00F0, 0x00F1, 0x00F2, 0x00F3, 0x00F4, 0x00F5, 0x00F6, 0x00F7, 0x00F8, 0x00F9, 0x00FA, 0x00FB, 0x00FC, 0x00FD, 0x00FE, 0x00FF,
];

#[rustfmt::skip]
const CP1250: [u16; 128] = [
    0x20AC, 0x0081, 0x201A, 0x0083, 0x201E, 0x2026, 0x2020, 0x2021, 0x0088, 0x2030, 0x0160, 0x2039, 0x015A, 0x0164, 0x017D, 0x0179,
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x0098, 0x2122, 0x0161, 0x203A, 0x015B, 0x0165, 0x017E, 0x017A,
    0x00A0, 0x02C7, 0x02D8, 0x0141, 0x00A4, 0x0104, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x015E, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x017B,
    0x00B0, 0x00B1, 0x02DB, 0x0142, 0x00B4, 0x00B5, 0x00B6, 0x00B7, 0x00B8, 0x0105, 0x015F, 0x00BB, 0x013D, 0x02DD, 0x013E, 0x017C,
    0x0154, 0x00C1, 0x00C2, 0x0102, 0x00C4, 0x0139, 0x0106, 0x00C7, 0x010C, 0x00C9, 0x0118, 0x00CB, 0x011A, 0x00CD, 0x00CE, 0x010E,
    0x0110, 0x0143, 0x0147, 0x00D3, 0x00D4, 0x0150, 0x00D6, 0x00D7, 0x0158, 0x016E, 0x00DA, 0x0170, 0x00DC, 0x00DD, 0x0162, 0x00DF,
    0x0155, 0x00E1, 0x00E2, 0x0103, 0x00E4, 0x013A, 0x0107, 0x00E7, 0x010D, 0x00E9, 0x0119, 0x00EB, 0x011B, 0x00ED, 0x00EE, 0x010F,
    0x0111, 0x0144, 0x0148, 0x00F3, 0x00F4, 0x0151, 0x00F6, 0x00F7, 0x0159, 0x016F, 0x00FA, 0x0171, 0x00FC, 0x00FD, 0x0163, 0x02D9,
];

#[rustfmt::skip]
const ISO8859_2: [u16; 128] = [
    0x0080, 0x0081, 0x0082, 0x0083, 0x0084, 0x0085, 0x0086, 0x0087, 0x0088, 0x0089, 0x008A, 0x008B, 0x008C, 0x008D, 0x008E, 0x008F,
    0x0090, 0x0091, 0x0092, 0x0093, 0x0094, 0x0095, 0x0096, 0x0097, 0x0098, 0x0099, 0x009A, 0x009B, 0x009C, 0x009D, 0x009E, 0x009F,
    0x00A0, 0x0104, 0x02D8, 0x0141, 0x00A4, 0x013D, 0x015A, 0x00A7, 0x00A8, 0x0160, 0x015E, 0x0164, 0x0179, 0x00AD, 0x017D, 0x017B,
    0x00B0, 0x0105, 0x02DB, 0x0142, 0x00B4, 0x013E, 0x015B, 0x02C7, 0x00B8, 0x0161, 0x015F, 0x0165, 0x017A, 0x02DD, 0x017E, 0x017C,
    0x0154, 0x00C1, 0x00C2, 0x0102, 0x00C4, 0x0139, 0x0106, 0x00C7, 0x010C, 0x00C9, 0x0118, 0x00CB, 0x011A, 0x00CD, 0x00CE, 0x010E,
    0x0110, 0x0143, 0x0147, 0x00D3, 0x00D4, 0x0150, 0x00D6, 0x00D7, 0x0158, 0x016E, 0x00DA, 0x0170, 0x00DC, 0x00DD, 0x0162, 0x00DF,
    0x0155, 0x00E1, 0x00E2, 0x0103, 0x00E4, 0x013A, 0x0107, 0x00E7, 0x010D, 0x00E9, 0x0119, 0x00EB, 0x011B, 0x00ED, 0x00EE, 0x010F,
    0x0111, 0x0144, 0x0148, 0x00F3, 0x00F4, 0x0151, 0x00F6, 0x00F7, 0x0159, 0x016F, 0x00FA, 0x0171, 0x00FC, 0x00FD, 0x0163, 0x02D9,
];

// PDF 1.7 (ISO 32000-1) Annex D.2.
const PDFDOC_LOW: [u16; 8] = [
    0x02D8, 0x02C7, 0x02C6, 0x02D9, 0x02DD, 0x02DB, 0x02DA, 0x02DC,
];
#[rustfmt::skip]
const PDFDOC_HIGH: [u16; 33] = [
    0x2022, 0x2020, 0x2021, 0x2026, 0x2014, 0x2013, 0x0192, 0x2044, 0x2039, 0x203A, 0x2212, 0x2030, 0x201E, 0x201C, 0x201D, 0x2018,
    0x2019, 0x201A, 0x2122, 0xFB01, 0xFB02, 0x0141, 0x0152, 0x0160, 0x0178, 0x017D, 0x0131, 0x0142, 0x0153, 0x0161, 0x017E, 0xFFFD,
    0x20AC,
];

#[cfg(test)]
mod tests {
    use super::*;

    const ZOLW_UTF8: &str = "Zażółć gęślą jaźń ŁÓDŹ";

    #[test]
    fn windows_1250_polish() {
        // "Zażółć gęślą jaźń ŁÓDŹ" in Windows-1250.
        let b = b"Za\xbf\xf3\xb3\xe6 g\xea\x9cl\xb9 ja\x9f\xf1 \xa3\xd3D\x8f";
        assert_eq!(decode(b, Encoding::Windows1250), ZOLW_UTF8);
        assert_eq!(decode_guess(b), ZOLW_UTF8);
    }

    #[test]
    fn iso_8859_2_polish() {
        let b = b"Za\xbf\xf3\xb3\xe6 g\xea\xb6l\xb1 ja\xbc\xf1 \xa3\xd3D\xac";
        assert_eq!(decode(b, Encoding::Iso8859_2), ZOLW_UTF8);
    }

    #[test]
    fn utf16_with_bom() {
        let mut b = vec![0xFE, 0xFF];
        for u in "Łódź".encode_utf16() {
            b.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(decode_guess(&b), "Łódź");
    }

    #[test]
    fn xml_declaration_is_honoured() {
        let doc = b"<?xml version='1.0' encoding=\"ISO-8859-2\"?><a>\xb1</a>";
        assert_eq!(
            decode_xml(doc),
            "<?xml version='1.0' encoding=\"ISO-8859-2\"?><a>ą</a>"
        );
    }

    #[test]
    fn pdfdoc() {
        assert_eq!(decode(b"\x95\x9b\x84", Encoding::PdfDoc), "Łł—");
    }

    #[test]
    fn western_fallback() {
        assert_eq!(decode_guess(b"caf\xe9 \x93x\x94"), "café “x”");
    }
}
