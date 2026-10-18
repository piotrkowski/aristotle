// SPDX-License-Identifier: AGPL-3.0-or-later
//! CBZ page names, matching the bundled MuPDF reader's ASCII ordering.

use std::cmp::Ordering;

pub(crate) const METADATA_VERSION: u32 = 2;

pub(crate) fn is_page_name(name: &str) -> bool {
    // MuPDF's source/cbz/mucbz.c defines the reader's page extensions.
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "bmp"
            | "gif"
            | "hdp"
            | "j2k"
            | "jb2"
            | "jbig2"
            | "jp2"
            | "jpeg"
            | "jpg"
            | "jpx"
            | "jxr"
            | "pam"
            | "pbm"
            | "pgm"
            | "pkm"
            | "png"
            | "pnm"
            | "ppm"
            | "tif"
            | "tiff"
            | "wdp"
    )
}

pub(crate) fn page_order(a: &str, b: &str) -> Ordering {
    let (original_a, original_b) = (a, b);
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while let (Some(&ac), Some(&bc)) = (a.first(), b.first()) {
        let order = if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let (an, rest_a) = digits(a);
            let (bn, rest_b) = digits(b);
            a = rest_a;
            b = rest_b;
            an.len().cmp(&bn.len()).then_with(|| an.cmp(bn))
        } else {
            a = &a[1..];
            b = &b[1..];
            ac.to_ascii_uppercase().cmp(&bc.to_ascii_uppercase())
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len()
        .cmp(&b.len())
        .then_with(|| original_a.as_bytes().cmp(original_b.as_bytes()))
}

fn digits(bytes: &[u8]) -> (&[u8], &[u8]) {
    let end = bytes
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(bytes.len());
    let start = bytes[..end].iter().position(|&c| c != b'0').unwrap_or(end);
    (&bytes[start..end], &bytes[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_names_match_the_reader() {
        assert!(is_page_name("folder/page2.BMP"));
        assert!(is_page_name("page10.TIFF"));
        assert!(is_page_name("page.png"));
        assert!(!is_page_name("page.webp"));
        assert!(!is_page_name("ComicInfo.xml"));
        assert!(!is_page_name("folder.png/"));
    }

    #[test]
    fn numeric_and_case_insensitive_order_does_not_overflow() {
        assert_eq!(page_order("PAGE2.png", "page10.png"), Ordering::Less);
        assert_eq!(page_order("page02.png", "page2.png"), Ordering::Less);
        assert_eq!(page_order("PAGE2.PNG", "page2.png"), Ordering::Less);
        assert_eq!(page_order("a0.png", "a00.png"), Ordering::Less);
        assert_eq!(page_order("page2.png", "page2.png"), Ordering::Equal);
        assert_eq!(page_order("a.png", "é.png"), Ordering::Less);
        assert_eq!(page_order("a1/page2.png", "a2/page1.png"), Ordering::Less);
        assert_eq!(page_order("page2.png", "page2a.png"), Ordering::Less);
        assert_eq!(page_order("page.png", "page2.png"), Ordering::Less);
        assert_eq!(
            page_order(
                "page999999999999999999999.png",
                "page1000000000000000000000.png"
            ),
            Ordering::Less
        );
    }
}
