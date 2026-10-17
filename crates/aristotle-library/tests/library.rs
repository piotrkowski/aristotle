//! End-to-end tests over generated fixtures: tiny EPUBs, a PDF, a TXT and a
//! CBZ laid out like `/mnt/us/documents`, scanned through the public API.
//! No copyrighted books are needed; everything is built here.

use std::fs;
use std::path::{Path, PathBuf};

use aristotle_library::clippings;
use aristotle_library::cover::read_pgm;
use aristotle_library::index::{AnnotationKind, Locator, Progress, Status};
use aristotle_library::library::{Library, ScanOptions};
use aristotle_library::{Format, Query, SortKey, StatusFilter};

fn testdir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir()
        .join("aristotle-library-tests")
        .join(format!("it-{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Stored-only ZIP writer, enough for EPUB and CBZ fixtures.
fn zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data) in members {
        let off = out.len() as u32;
        let mut h = Vec::new();
        h.extend_from_slice(&[20, 0, 0, 0x08, 0, 0, 0, 0, 0, 0]);
        h.extend_from_slice(&crc32(data).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&[0, 0]);
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&h);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        cd.extend_from_slice(b"PK\x01\x02\x14\x00");
        cd.extend_from_slice(&h);
        cd.extend_from_slice(&[0; 10]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&(members.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

fn png(w: u32, h: u32, shade: u8) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        let data: Vec<u8> = (0..w * h)
            .flat_map(|i| [shade, (i % 256) as u8, 255 - shade, 255])
            .collect();
        wr.write_image_data(&data).unwrap();
    }
    out
}

const CONTAINER: &[u8] = br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

/// EPUB 3 with a cover-image item and a series collection.
fn epub3(
    title: &str,
    author: &str,
    file_as: &str,
    series: Option<(&str, u32)>,
    cover_shade: u8,
) -> Vec<u8> {
    let series_meta = series
        .map(|(s, i)| {
            format!(
                r##"<meta property="belongs-to-collection" id="c">{s}</meta><meta refines="#c" property="collection-type">series</meta><meta refines="#c" property="group-position">{i}</meta>"##
            )
        })
        .unwrap_or_default();
    let opf = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:uuid:{title}</dc:identifier>
<dc:title>{title}</dc:title>
<dc:creator id="a">{author}</dc:creator><meta refines="#a" property="file-as">{file_as}</meta>
<dc:language>pl</dc:language>
{series_meta}
</metadata>
<manifest>
<item id="img" href="images/cover.png" media-type="image/png" properties="cover-image"/>
<item id="t" href="text.xhtml" media-type="application/xhtml+xml"/>
</manifest>
<spine><itemref idref="t"/></spine>
</package>"##
    );
    let cover = png(60, 90, cover_shade);
    zip(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/images/cover.png", &cover),
        (
            "OEBPS/text.xhtml",
            b"<html><body><p>Tekst.</p></body></html>",
        ),
    ])
}

/// EPUB 2 in ISO-8859-2 with `<meta name="cover">`, calibre series, and the
/// cover reachable only through an XHTML page with an SVG `<image>`.
fn epub2_latin2() -> Vec<u8> {
    let mut opf: Vec<u8> = br#"<?xml version="1.0" encoding="iso-8859-2"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
<dc:title>"#
        .to_vec();
    opf.extend_from_slice(b"\xa3\xf3d\xbc noc\xb1"); // "Łódź nocą" in ISO-8859-2
    opf.extend_from_slice(
        br#"</dc:title>
<dc:creator opf:role="aut" opf:file-as="Nowak, Anna">Anna Nowak</dc:creator>
<meta name="calibre:series" content="Miasta"/><meta name="calibre:series_index" content="1"/>
<meta name="cover" content="cpage"/>
</metadata>
<manifest><item id="cpage" href="cover.xhtml" media-type="application/xhtml+xml"/></manifest>
<spine><itemref idref="cpage"/></spine>
</package>"#,
    );
    let page = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body>&nbsp;<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="img/okladka.png"/></svg></body></html>"#;
    let cover = png(30, 40, 10);
    zip(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/cover.xhtml", page),
        ("OEBPS/img/okladka.png", &cover),
    ])
}

fn pdf(title: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = vec![];
    for body in [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [] /Count 12 >>".into(),
        format!("<< /Title ({title}) /Author (Jan Kowalski) >>"),
    ] {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", offs.len()).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R /Info 3 0 R >>\nstartxref\n{x}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// A documents folder like the device's: author folders, sidecars,
/// clutter that must be skipped.
fn make_library(root: &Path) {
    write(
        &root.join("Platon/Uczta.epub"),
        &epub3("Uczta", "Platon", "Platon", Some(("Dialogi", 2)), 200),
    );
    write(
        &root.join("Platon/Fedon.epub"),
        &epub3("Fedon", "Platon", "Platon", Some(("Dialogi", 1)), 40),
    );
    write(&root.join("Anna Nowak/lodz.epub"), &epub2_latin2());
    write(
        &root.join("Jan Kowalski/wyklad.pdf"),
        &pdf("Liczby zespolone"),
    );
    write(
        &root.join("Bolesław Prus/Lalka.txt"),
        "Tom pierwszy\n".as_bytes(),
    );
    write(
        &root.join("Komiksy/Tytus.cbz"),
        &zip(&[("002.png", &png(10, 10, 0)), ("001.png", &png(20, 30, 128))]),
    );
    // Things that must not become books.
    write(&root.join("My Clippings.txt"), b"x");
    write(
        &root.join(".hidden/Ukryta.epub"),
        &epub3("Ukryta", "X", "X", None, 0),
    );
    write(
        &root.join("Platon/Uczta.sdr/Uczta.epub"),
        &epub3("Sidecar", "X", "X", None, 0),
    );
    write(
        &root.join("dictionaries/Słownik.epub"),
        &epub3("Słownik", "X", "X", None, 0),
    );
    write(&root.join("Platon/notes.zip"), &zip(&[("a.txt", b"x")]));
    write(&root.join("Platon/broken.epub"), b"PK\x03\x04garbage");
}

fn by_title<'a>(lib: &'a Library, title: &str) -> &'a aristotle_library::BookRecord {
    lib.books()
        .iter()
        .find(|b| b.meta.title == title)
        .unwrap_or_else(|| panic!("no book {title}"))
}

#[test]
fn scan_describes_every_format() {
    let base = testdir("formats");
    let root = base.join("documents");
    make_library(&root);
    let mut lib = Library::open(base.join("data")).unwrap();
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!(
        lib.books().len(),
        6,
        "{:#?}",
        lib.books().iter().map(|b| &b.file.path).collect::<Vec<_>>()
    );
    assert_eq!((r.added, r.ignored), (6, 1), "the plain zip is ignored");
    assert_eq!(r.errors.len(), 1, "broken.epub is reported: {:?}", r.errors);
    assert!(r.errors[0].0.ends_with("broken.epub"));

    let uczta = by_title(&lib, "Uczta");
    assert_eq!(uczta.format, Format::Epub);
    assert_eq!(
        uczta
            .meta
            .series
            .as_ref()
            .map(|s| (s.name.as_str(), s.index)),
        Some(("Dialogi", Some(2.0)))
    );
    let thumb = read_pgm(&lib.cover_path(uczta).expect("thumbnail")).unwrap();
    assert_eq!((thumb.width, thumb.height), (200, 300));
    assert!(thumb.pixels.iter().all(|v| v % 17 == 0));

    let lodz = by_title(&lib, "Łódź nocą");
    assert_eq!(lodz.meta.authors[0].sort, "Nowak, Anna");
    assert!(
        lib.cover_path(lodz).is_some(),
        "cover through the XHTML cover page"
    );

    let pdf = by_title(&lib, "Liczby zespolone");
    assert_eq!((pdf.format, pdf.meta.pages), (Format::Pdf, Some(12)));

    let txt = by_title(&lib, "Lalka");
    assert_eq!(txt.format, Format::Txt);
    assert_eq!(
        txt.meta.authors[0].name, "Bolesław Prus",
        "author from the folder"
    );

    let cbz = by_title(&lib, "Tytus");
    assert_eq!((cbz.format, cbz.meta.pages), (Format::Cbz, Some(2)));
    let t = read_pgm(&lib.cover_path(cbz).unwrap()).unwrap();
    assert_eq!(t.height, 300, "first page by name (20 x 30) is the cover");

    // Same cover bytes, same thumbnail file.
    lib.save().unwrap();
    let again = Library::open(base.join("data")).unwrap();
    assert_eq!(again.books().len(), 6);
}

#[test]
fn rescans_are_incremental_and_state_follows_the_book() {
    let base = testdir("incremental");
    let root = base.join("documents");
    make_library(&root);
    let data = base.join("data");
    let mut lib = Library::open(&data).unwrap();
    lib.scan(&root, &ScanOptions::default()).unwrap();
    let id = by_title(&lib, "Uczta").id.clone();
    lib.mark_opened(&id, 1_000);
    lib.set_progress(
        &id,
        Progress {
            fraction: 0.4,
            locator: Locator::Epub {
                spine: 1,
                offset: 10,
            },
            updated: 1_000,
            source: None,
            page: None,
        },
    );
    lib.save().unwrap();

    // Unchanged: nothing is opened again.
    let mut lib = Library::open(&data).unwrap();
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!((r.unchanged, r.added, r.updated, r.ignored), (6, 0, 0, 1));
    assert!(r.parse_time.is_zero());

    // Move to another folder and rename: same ID, state kept.
    fs::create_dir_all(root.join("Inni")).unwrap();
    fs::rename(
        root.join("Platon/Uczta.epub"),
        root.join("Inni/Symposion.epub"),
    )
    .unwrap();
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!((r.moved, r.added, r.removed), (1, 0, 0));
    let b = lib.book(&id).unwrap();
    assert_eq!(b.file.path, "Inni/Symposion.epub");
    assert_eq!((b.status, b.last_opened), (Status::Reading, Some(1_000)));
    assert_eq!(b.progress.as_ref().unwrap().fraction, 0.4);

    // Delete, rescan, copy back: the state is restored from `removed`.
    let bytes = fs::read(root.join("Inni/Symposion.epub")).unwrap();
    fs::remove_file(root.join("Inni/Symposion.epub")).unwrap();
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!(r.removed, 1);
    assert!(lib.book(&id).is_none());
    write(&root.join("Platon/Uczta.epub"), &bytes);
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!(r.restored, 1);
    assert_eq!(
        lib.book(&id).unwrap().progress.as_ref().unwrap().fraction,
        0.4
    );

    // Content replaced in place: new ID, state carried over by path.
    write(
        &root.join("Platon/Uczta.epub"),
        &epub3("Uczta", "Platon", "Platon", Some(("Dialogi", 2)), 90),
    );
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!(r.updated, 1);
    let b = by_title(&lib, "Uczta");
    assert_ne!(b.id, id);
    assert_eq!(b.last_opened, Some(1_000));
}

#[test]
fn queries_over_a_scanned_library() {
    let base = testdir("query");
    let root = base.join("documents");
    make_library(&root);
    let mut lib = Library::open(base.join("data")).unwrap();
    lib.scan(&root, &ScanOptions::default()).unwrap();
    let fedon = by_title(&lib, "Fedon").id.clone();
    lib.set_status(&fedon, Status::Finished);
    lib.mark_opened(&fedon, 50);

    let titles = |q: &Query| {
        lib.query(q)
            .iter()
            .map(|b| b.meta.title.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles(&Query {
            sort: SortKey::Series,
            formats: vec![Format::Epub],
            ..Default::default()
        }),
        ["Fedon", "Uczta", "Łódź nocą"]
    );
    assert_eq!(
        titles(&Query {
            search: Some("LODZ".into()),
            ..Default::default()
        }),
        ["Łódź nocą"]
    );
    assert_eq!(
        titles(&Query {
            search: Some("prus".into()),
            ..Default::default()
        }),
        ["Lalka"]
    );
    assert_eq!(
        titles(&Query {
            status: Some(StatusFilter::Finished),
            ..Default::default()
        }),
        ["Fedon"]
    );
    assert_eq!(
        titles(&Query::default())[0],
        "Fedon",
        "recently opened first"
    );
    assert_eq!(
        lib.query(&Query {
            status: Some(StatusFilter::Unread),
            ..Default::default()
        })
        .len(),
        5
    );
}

#[test]
fn clippings_attach_to_books_once() {
    let base = testdir("clippings");
    let root = base.join("documents");
    make_library(&root);
    let mut lib = Library::open(base.join("data")).unwrap();
    lib.scan(&root, &ScanOptions::default()).unwrap();
    let file = "\u{FEFF}Uczta (Platon)\r\n- Your Highlight on page 3 | location 40-42 | Added on Friday, 3 September 2021 07:05:09\r\n\r\nEros jest filozofem.\r\n==========\r\n\
Lodz noca (Nowak, Anna)\r\n- Twoja notatka w lokalizacji 12 | Dodano: piątek, 3 września 2021 07:05:09\r\n\r\nNotatka\r\n==========\r\n\
Inna książka (Ktoś)\r\n- Your Bookmark at location 1 | Added on Friday, 3 September 2021 07:05:09\r\n\r\n\r\n==========\r\n";
    let parsed = clippings::parse(file.as_bytes());
    assert_eq!(parsed.clippings.len(), 3);
    let r = lib.import_clippings(&parsed.clippings);
    assert_eq!((r.matched, r.added, r.unmatched.len()), (2, 2, 1));
    let r = lib.import_clippings(&parsed.clippings);
    assert_eq!((r.added, r.duplicates), (0, 2));
    let u = by_title(&lib, "Uczta");
    assert_eq!(u.annotations.len(), 1);
    assert_eq!(u.annotations[0].kind, AnnotationKind::Highlight);
    assert_eq!(
        u.annotations[0].text.as_deref(),
        Some("Eros jest filozofem.")
    );
    assert_eq!(
        by_title(&lib, "Łódź nocą").annotations[0].kind,
        AnnotationKind::Note
    );
}

#[test]
fn koreader_sidecar_is_imported_for_new_books() {
    let base = testdir("koreader");
    let root = base.join("documents");
    make_library(&root);
    write(
        &root.join("Platon/Fedon.sdr/metadata.epub.lua"),
        b"return {\n    [\"last_xpointer\"] = \"/body/DocFragment[3]/body/p[2]/text().0\",\n    [\"percent_finished\"] = 1,\n    [\"summary\"] = {\n        [\"status\"] = \"complete\",\n    },\n}\n",
    );
    let mut lib = Library::open(base.join("data")).unwrap();
    let r = lib.scan(&root, &ScanOptions::default()).unwrap();
    assert_eq!(r.sidecars_imported, 1);
    let f = by_title(&lib, "Fedon");
    assert_eq!(f.status, Status::Finished);
    let p = f.progress.as_ref().unwrap();
    assert_eq!((p.fraction, p.source.as_deref()), (1.0, Some("koreader")));
    assert!(
        matches!(&p.locator, Locator::XPointer { xpointer } if xpointer.starts_with("/body/DocFragment[3]"))
    );
}
