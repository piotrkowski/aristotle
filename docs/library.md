# Book library

`crates/aristotle-library` finds the books on the device, describes them and keeps their reading state. It also imports the state that the stock reader and KOReader leave next to each book. The crate is pure Rust with no C dependencies. `aristotle-library-cli` drives it on the host or over SSH on the Kindle.

Source marks: **[fw]** read from firmware 5.13.6, **[dev]** verified on files from our device, **[host]** verified on the host with real files, **[KOReader]** and **[calibre]** borrowed from those projects, **[inference]** not yet confirmed.

<img src="images/library.png" width="325" alt="The library on the device: the book to continue at the top, then the Books, Folders and Series views, the shelves All, Reading, New and Finished, and a page of books">

## What it indexes

The library root is `documents/` (`/mnt/us/documents` on the device). Formats are detected by content, so a PDF named `.epub` is indexed as a PDF.

| Format | Detected by | Metadata | Cover |
| --- | --- | --- | --- |
| EPUB 2/3 | ZIP with an `application/epub+zip` `mimetype` or a `META-INF/container.xml` | OPF | yes |
| MOBI (KF7, joint KF7/KF8) | PalmDB `BOOKMOBI`, MOBI header version below 8 | record 0, EXTH | EXTH 201 |
| AZW3 (KF8 only) | `BOOKMOBI`, MOBI header version 8 | record 0, EXTH | EXTH 201 |
| PalmDOC | `TEXtREAd` | database name | no |
| PDF | `%PDF-` in the first 1 KiB | Info dictionary, page count | no |
| CBZ | ZIP named `.cbz` or `.zip`, at least half of it images | `ComicInfo.xml`, page count | first image |
| TXT | `.txt`, no control bytes in the first 1 KiB (or a UTF-16 BOM) | file and folder name | no |

Only files with a book extension (`epub kepub mobi azw azw3 prc pdb pdf txt cbz zip`) or none are opened. The scan skips hidden entries, `*.sdr` folders, folders named `dictionaries`, `My Clippings.txt` and symlinks to folders. KFX, CBR, CB7, FB2, DjVu and Topaz are not supported. Encrypted MOBI files are listed with `drm: true`, and the reader does not open them. Kindle dictionaries (EXTH 531) stay out of the library.

## Metadata

A book always gets a title. The sources, highest priority first:

1. Metadata embedded in the file.
2. The folder. Books on the device sit in `documents/<Author>/`, so a book with no embedded author takes the name of its first folder below the root.
3. The file name, for the title. Underscores become spaces, and a shop product code after `__` is dropped: `Obrona_Sokratesa__p1234567890.epub` becomes "Obrona Sokratesa". Such titles carry `title_source: "filename"`.

### Author folders

MOBI, AZW3 and PDF have no sort names, and their author fields are often inverted or run several people together. A MOBI may say "Żółkiewska Jadwiga Łęcki Tomasz" in one field while its folder is "Lecki Tomasz, Zolkiewska Jadwiga". For these formats the folder can repair the authors. It is used only if its words are the embedded words, ignoring case, diacritics and punctuation, and if it splits one field into several authors or shows that a name is inverted. "Schopenhauer Arthur" then becomes "Arthur Schopenhauer", sort "Schopenhauer, Arthur".

If one side of a comma is a single word, the folder names one person as "Last, First". Otherwise commas separate people, and a list of common given names decides which end of each name is the given name. When that list does not decide, several authors are read surname first and a single name is left alone. Diacritics always come from the book. EPUB is never repaired, because its `file-as` is authoritative.

### Per format

EPUB metadata comes from the OPF, matched by local name because real files misuse namespaces. Authors are the `dc:creator` entries with role `aut` or none, and editors count only when there is no author. The sort name is `file-as`, else derived from "Given Surname". Series come from EPUB 3 `belongs-to-collection`, with `calibre:series` as the fallback. The cover is the `cover-image` item, else `<meta name="cover">`, else the guide's cover page, else an image or first page named like a cover.

MOBI and AZW3 read only the record table, record 0 and the cover record. The title is EXTH 503, else the full name in record 0. Authors are EXTH 100, split on `&` and `;`, and "Surname, Given" is shown as "Given Surname". calibre puts the book's UUID in the ASIN fields, so a UUID there is filed as `uuid`.

PDF gives the Info dictionary and the page count. The parser reads xref tables and streams, object streams and `/Prev` chains, and scans the body when the xref is damaged. Placeholder titles such as "Microsoft Word - x.docx" give way to the file name. An encrypted PDF gives only its page count and is not marked as DRM, because owner-password PDFs open fine. CBZ reads `ComicInfo.xml`, and TXT only its names.

Legacy encodings (Windows-1250 and -1252, ISO-8859-2, Latin-1, UTF-16, PDFDocEncoding) use small hand-written tables. An undeclared encoding is guessed as Windows-1250 when the bytes hold Polish letters that exist only there.

## Identity and the index

A book's ID is KOReader's partial MD5 **[KOReader `util.partialMD5`]**: up to twelve 1 KiB samples at offsets 0 and 1024 << 2i for i = 0 to 10. It survives renames and moves, and KOReader uses the same hash for its sidecars and kosync. Identical copies get `-2`, `-3` suffixes.

The index is `<data>/library.json`, with the data folder at `/mnt/us/.aristotle` on the device (or `--data`, `$ARISTOTLE_DATA`). It is written to a temporary file, synced and renamed, as the stock reader saves its sidecars **[fw]**. JSON stays loadable when fields are added and can be repaired over SSH.

A book record holds the file stamp, metadata, cover key, status, progress, per-book settings and annotations. The status is `unread`, `reading` or `finished`. Opening a book moves it to `reading`, and `finished` is set explicitly or by an import. Progress has an approximate `fraction` and a locator that is exact for the renderer that wrote it:

| Locator | Position |
| --- | --- |
| `epub` | spine index and character offset |
| `kindle` | MOBI/AZW3 byte position, the stock reader's unit |
| `page` | PDF or CBZ page index |
| `text` | TXT byte offset |
| `xpointer` | KOReader's XPointer, kept verbatim |

A re-scan opens no file whose size and mtime match the index, and non-book files are remembered in `ignored`. Reading state follows the book ID. A changed file keeps the state of its path, and a new path with the ID of a vanished one is a move. Deleted books stay in `removed` (up to 500), so a book copied back resumes where it was. A damaged file is reported and does not stop the scan.

Covers become thumbnails 200 px wide and at most 320 px high, dithered to the panel's 16 gray levels. They are PGM files in `<data>/covers/`, named by the MD5 of the cover bytes, so duplicate editions share one file.

## Shelves, folders and series

The home screen shows the book to continue at the top. It is the book opened last in Aristotle. Until a book is opened here, it is the one whose progress changed last, which may come from the stock reader. Below it are three views.

- **Books** lists all books. Shelf chips narrow it to All, Reading, New (never opened) or Finished.
- **Folders** mirrors `documents/`. A book's folder is the directory of its path, and nested folders nest. A folder opens as its own shelf, with subfolders first and then its books.
- **Series** has one shelf per series name. Its books are in series order.

Lists turn pages and never scroll. The sort button cycles through recent, title and author. Search and sorting fold case and diacritics, `ł` included, so "lodz noca" finds "Łódź nocą". Each query word must occur in the title, an author or the series. Polish collation, where `ł` follows `l`, would need locale data and is not used.

## Stock files: sidecars and My Clippings.txt

Aristotle reads the Kindle's reading-position sidecars (KRDS), so positions and highlights from the stock reader carry over to Aristotle. Highlights made in Aristotle go to `My Clippings.txt` in the stock format. The format is described here in our own words, from public documentation (jhowell's KRDS notes on MobileRead, [krdsrw](https://github.com/midrare/krdsrw), `kindle_formats`) and from files on the device, for interoperability. No Amazon code is included.

### Sidecar folders **[fw]**

For `<dir>/<name>.<ext>` the stock reader keeps two files in `<dir>/<name>.sdr/`. The book class in `ReaderSDK.jar` decides their extensions:

| Book class | Positions ("freq") | Annotations, settings ("rare") |
| --- | --- | --- |
| MOBI 7 (`MobiBook`) | `.mbs` | `.mbp1` |
| KF8 / AZW3 (`HTMLBook`) | `.azw3f` | `.azw3r` |
| KFX (`YJBook`) | `.yjf` | `.yjr` |
| PDF | `.pdt` | `.pds` |
| Topaz | `.tas` | `.tal` |

The folder may also hold the `.apnx` page map and legacy `.han` and `.tan` files. Firmware 5.x reads an old `.mbp` once, migrates it into the KRDS files and deletes it. KOReader's `metadata.<ext>.lua` lives in the same folders. Both sideloaded `.mobi` books on our device got `.mbs` and `.mbp1` **[dev]**.

### KRDS

Both files use the big-endian serialisation of Java's `DataOutputStream`. The layout matches what the firmware expects **[fw]** and, byte for byte, four `.mbs` and `.mbp1` files from our device **[dev]**.

```text
00 00 00 00 00 1A B1 26        magic (untagged)
02 <i64 = 1>                   version
01 <i32 n>                     number of top-level objects
object := FE <null flag> <u16 len + modified UTF-8 name> value* FF
value  := 00 bool | 01 i32 | 02 i64 | 03 utf | 04 f64 | 05 i16 | 06 f32 | 07 i8 | 09 char | object
utf    := 01 (null) | 00 <u16 len> <modified UTF-8>
```

| Object | Layout | Source |
| --- | --- | --- |
| `lpr` (last page read) | byte 2, utf position, long epoch ms | [fw][dev] |
| `fpr` (furthest page read) | utf position, long ms, long local-time offset, utf country, utf device | [fw][dev] |
| `annotation.cache.object` | int n, then n times an int type (0 bookmark, 1 highlight, 2 note, 3 clip) and a `saved.avl.interval.tree` of `annotation.personal.*` objects | [fw] |

Each annotation object holds utf start, utf end, long created ms, long modified ms and a utf tags list. Notes add the note text. The highlighted text itself is not stored. Our device samples had no annotations, so this layout is not verified on the device.

A device `lpr` position looks like `"7510:7510:11:REFUQQAA…"`. The first field is a byte offset into the uncompressed MOBI text **[dev]**, and AZW3 uses the same unit **[fw]**. The rest is an unknown number and 132 bytes of base64 render state. Through the page maps, `7510` is page 4 of 180 and `49114` page 22 of 1,732, which matches the 2% and 1% the stock UI showed **[dev]**. PDF positions are `a b c nRects [x y w h]…` and KFX positions `serialized:positionId` **[fw]**.

The device files also hold reading-speed statistics and a copy of the page map (`apnx.key`), but no `font.prefs`, `erl` or `sync_lpr`. Those are presumably written only when used **[inference]**.

The `.apnx` page map, as calibre writes it **[dev]**:

```text
u16 1, u16 1, u32 offset of the page header, u32 length of JSON 1
JSON 1   {"contentGuid":"…","asin":"…","cdeType":"EBOK","fileRevisionId":"1"}
header   u16 1, u16 length of JSON 2, u16 page count, u16 32
JSON 2   {"asin":"…","pageMap":"(1,a,1)"}
u32 × page count   start position of each page
```

`pageMap` lists `(first page, a|r, first number)` groups for arabic or roman numbering.

### What is imported

Sidecars are imported on a scan for new books, or for all books with `aristotle-library-cli import-sidecars`. Progress is taken only if it is newer than the record's, so a re-import never rewinds a book read further in Aristotle.

- MOBI and AZW3 progress comes from `lpr`. The fraction is the position over the text length in record 0. The `.apnx` page map gives the page label, and the fraction when the text length is unknown. A fraction of 0.99 or more marks the book finished.
- Bookmarks, highlights and notes keep their positions, creation time and note text. Kindle locations are position / 150 + 1, calibre's constant **[calibre]**. That 5.x uses the same is an **[inference]**.
- From KOReader's `metadata.<ext>.lua`: `percent_finished`, `last_xpointer` or `last_page`, and `summary.status` (`complete` means finished). Its `partial_md5_checksum` must match the book ID.
- Not imported: `fpr`, PDF and KFX positions, font preferences and KOReader highlights.

Unit tests build sidecars with the device layouts and invented values. An ignored test parses real copies: `ARISTOTLE_SDR_SAMPLES=$ARISTOTLE_CACHE/sdr-samples cargo test -p aristotle-library device_samples -- --ignored --nocapture`.

### My Clippings.txt **[fw]**

The stock reader writes `documents/My Clippings.txt` through `Clipping` in `ReaderSDK-impl.jar`. The file is UTF-8 with CRLF line ends and a BOM only when it is created. An entry:

```text
{title} ({author})
- {label} | Added on {date FULL} {time MEDIUM}

{text}
==========
```

The label is "Your Highlight at location 60-63", "Your Highlight on page 12 | location 180-181", or "Your Highlight on page 12" for PDF. Dates use ICU patterns, such as en_GB `EEEE, d MMMM y HH:mm:ss`. Firmware 5.13.6 has labels for en, en_GB, de, es, fr, it, ja, nl, pt, ru and zh, and none for Polish. The parser matches keyword stems in about a dozen languages, Polish guesses included, and reads dates in any field order. It parsed all 498 entries of a real en_GB file **[host]**.

`aristotle-library-cli clippings FILE --import` attaches clippings to books by folded title, then by prefix, with the author breaking ties. Importing twice adds nothing. Clippings carry no position, so they never set progress.

## Export

The reader exports its own highlights in two forms (`docs/reader.md`, "Highlights, bookmarks and export").

New highlights are appended to `My Clippings.txt` in the stock template, with the en_GB labels whatever the UI language. Entries already in the file are skipped, so repeated exports add nothing. A note becomes a "Your Note" entry at the highlight's first location, written before it, where common clippings parsers expect it. Bookmarks are not exported, and deleting a highlight leaves it in the file, as on the stock Kindle. The reader writes the file only while the setting "Save highlights to My Clippings.txt" is on, which is the default.

Markdown goes to `/mnt/us/aristotle-notes/`, one file per book (`<Title> — <Author>.md`), rewritten each time. It has front matter for Obsidian and one quote per highlight with chapter, location, date and note. Its labels follow the UI language.

## CLI

`aristotle-library-cli` has the commands `scan`, `list` (with sort, format, status and search filters), `show`, `clippings`, `import-sidecars` and `thumb`, which exports a thumbnail as PNG. Run it without arguments for the full usage.

## Performance

A 2-core host, release build, 28 MB sample of 25 EPUB, 4 MOBI, 1 PDF and 1 TXT in author folders **[host]**:

| Run | Time |
| --- | --- |
| Cold scan with thumbnails (31 books, 29 covers) | 200–340 ms, of which covers 187–307 ms |
| Cold scan without covers | 1.8 ms |
| Re-scan, nothing changed | 0.1 ms |
| Index size | 16.8 KB JSON; thumbnails 1.7 MB |

Covers cost about 6–10 ms each on the host. The PW4's Cortex-A9 is roughly ten times slower, so a first scan should take a few seconds **[inference]**.

## Network: progress sync and catalogs

Both are off until set up, in Settings › Network. The app opens connections only to the sync server and the catalogs given there, from worker threads; reading never waits for the network. Requests and redirects to a host the device's Wi-Fi policy does not allow fail at once with "<host> is not in the allowed hosts" ([device/README.md](../device/README.md#network)). Saving a server or catalog adds `host:port   # aristotle:sync` or `# aristotle:opds` to `net-allow` through `aristotlectl net allow add`, and removing it takes that line out again; lines without the tag are never touched. Code: `crates/aristotle-net` (HTTP over `ureq`, Basic and Digest auth, KOSync, OPDS), `aristotle_library::kosync` (positions), `crates/aristotle/src/{net,sync,catalogs}.rs`.

### Settings files

Until the on-screen keyboard can type them, the settings come from two files next to the app (`/mnt/us/aristotle/` for the installed bundle). They are read at start when they changed, and again with "Load sync.json" or "Load catalogs.json". A password in them is stored and then removed from the file.

```json
// sync.json
{ "server": "http://192.168.1.10:17200", "username": "anna", "password": "…", "auto": true }

// catalogs.json
{ "catalogs": [
  { "name": "calibre", "url": "http://192.168.1.10:8080/opds", "username": "anna", "password": "…" },
  { "name": "Project Gutenberg", "url": "https://m.gutenberg.org/ebooks.opds/" } ] }
```

They end up in `<data>/accounts.json`, created owner-only (0600). On the device that folder is on the FAT userstore, which has no permissions, so anyone with the device on a USB cable can read it. For sync only `md5(password)` is kept, which still works as the password for that one server; catalog passwords are kept as given, because Basic and Digest need them. Catalogs are matched by name: a listed one replaces ours (keeping the stored password when the file has none), new ones are added. The screens are built for a `TextInput` request per field (server, user name, password, a catalog's address, search terms); until the keyboard lands, tapping such a row says which file to edit.

### Progress sync (KOSync)

The protocol is KOReader's (`kosync.koplugin`) against any compatible server, such as `koreader/kosync` [KOReader]. The document ID is the book ID without its `-2` suffix, KOReader's partial MD5. A random 32-digit device ID is made once; the device name is "Aristotle".

| Format | Sent as `progress` | A pulled position |
| --- | --- | --- |
| EPUB | `/body/DocFragment[N]/body`, the chapter start | chapter N, then KOReader's percentage within it if it falls inside |
| PDF, CBZ | the 1-based page | that page |
| MOBI, AZW3, TXT | nothing: KOReader lays these out in its own DOM, and a made-up XPointer would take its reader to page 1 | the percentage |

N counts the `<itemref>`s whose `idref` names a manifest item with an `href`, placeholders for unreadable items included, as crengine does (`epubfmt.cpp`) [KOReader]. Plato skips items missing from the archive, so the two numberings differ after such an item; `Fragments` maps between them from the OPF. Element paths after `DocFragment[N]` are not resolved yet.

- **Open:** with "Sync automatically" on, the server's position is fetched (5 s timeout). If another device stored it after the book was last read here and it is on another page, a popup asks "Kobo is at 62%, in "Chapter". Go there?". The page never moves without asking.
- **Close** (back to the library, the screensaver, quitting): the position is sent if it changed since the book opened, and only in the forms above.
- Errors (refused login, an unreachable server) show on the sync screen and never interrupt reading. Failed pushes are not queued yet.

### Catalogs (OPDS 1.2)

Atom feeds with navigation and acquisition entries, `next`/`previous` paging (the pager arrows ask for the server's next page at the end of the list), and search through an OpenSearch description or a direct `{searchTerms}` template (calibre's). Links resolve against `xml:base` (Project Gutenberg). EPUB, AZW3, MOBI, PDF, CBZ and TXT links are offered, EPUB first; shop, loan and indirect acquisitions are skipped. Feeds are limited to 2 MB, and no covers are shown.

A download goes to `documents/<catalog name>/<title> - <author>.<ext>`, at most 200 MB and never filling the userstore past its last 20 MB. It streams into a hidden `.<name>.part` file, is synced and renamed, never over an existing file (` (2)` is added, compared without case as FAT does); a failure or Back removes the part file. The library then scans, so the book appears with its ID and "Read now" opens it.

HTTPS needs the `tls` feature (rustls with ring and the webpki root certificates), on in the release build (`APP_FEATURES` in `tools/build-render.sh`). It adds about 611 KB to the app; plain HTTP is enough for a server on the local network. A certificate that is not valid at the device's time reads as "Check the date and time".

## Known limitations

- Stock annotations are not verified on real data. Neither device sample had bookmarks, highlights or notes, and no KF8 sidecar (`.azw3f`, `.azw3r`) has been seen yet.
- Polish clippings labels are a guess.
- PDFs get no thumbnail, since that needs page rendering. GIF and WebP covers are not decoded.
- KOReader highlights are not imported, only progress and status.
- A MOBI author field that is inverted or merged stays as it is unless the author folder repairs it.
