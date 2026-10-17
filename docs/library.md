# Book library

`crates/aristotle-library` indexes books and stores reading state. It imports stock Kindle sidecars and KOReader progress. The crate is pure Rust. `aristotle-library-cli` runs it on the host or device.

Source marks: [fw] means firmware 5.13.6, [dev] means device files, [host] means host tests, and [KOReader] or [calibre] names an external source. [inference] means unconfirmed.

<img src="images/library.png" width="325" alt="Library with continue-reading card, views, shelves, and books">

## What it indexes

The device book root is `/mnt/us/documents`. Detection uses file contents, not just extensions.

| Format | Detection | Metadata | Cover |
| --- | --- | --- | --- |
| EPUB | ZIP with EPUB mimetype or `META-INF/container.xml` | OPF | Embedded cover |
| MOBI, AZW3 | PalmDB `BOOKMOBI` | Record 0 and EXTH | EXTH 201 |
| PalmDOC | `TEXtREAd` | Database name | None |
| PDF | `%PDF-` in the first 1 KiB | Info dictionary and page count | None |
| CBZ | `.cbz` or `.zip` with at least half its entries images | `ComicInfo.xml` and page count | First image |
| TXT | `.txt` with text bytes or UTF-16 BOM | File and folder names | None |

The scan opens known book extensions or extensionless files. It skips hidden entries, `.sdr` folders, `dictionaries`, `My Clippings.txt`, and directory symlinks. Kindle dictionaries stay out of the library. Encrypted MOBI is listed with `drm: true` but cannot be read.

KFX, CBR, CB7, FB2, DjVu, and Topaz are unsupported. Index support does not imply reader support ([Reader formats](reader.md#formats-and-positions)).

## Metadata

Metadata comes from the book first, then its author folder, then the file name. File-name titles replace underscores with spaces and drop shop codes after `__`. They carry `title_source: "filename"`.

### Author folders

For MOBI, AZW3, and PDF, a matching author folder can split merged names or correct surname-first order. Matching ignores case, diacritics, and punctuation. The book supplies the displayed diacritics. EPUB authors are not repaired because their `file-as` value takes priority.

### Per format

- EPUB reads OPF authors, sort names, series, and covers. EPUB 3 series take priority over `calibre:series`.
- MOBI and AZW3 read record 0, EXTH metadata, and the cover record without loading the text.
- PDF reads Info metadata and page counts, including damaged cross-reference fallbacks. Placeholder titles give way to the file name.

## Identity and the index

Book IDs use KOReader's partial MD5 and survive moves and renames [KOReader]. Identical copies get `-2`, `-3`, and further suffixes.

`library.json` lives in the data directory, `/mnt/us/.aristotle` on the device. Writes use a temporary file, sync, and rename. Records contain metadata, file stamps, cover keys, status, progress, settings, and annotations.

Status is `unread`, `reading`, or `finished`. Opening a book sets `reading`. Completion is explicit or imported. Progress stores a fraction and a renderer-specific locator.

| Locator | Position |
| --- | --- |
| `epub` | Spine index and character offset |
| `kindle` | MOBI or AZW3 byte position |
| `page` | PDF or CBZ page index |
| `text` | TXT byte offset |
| `xpointer` | KOReader position, kept verbatim |

Unchanged size and mtime avoid reopening a file. State follows the ID across moves, or the path when file contents change. Up to 500 removed records preserve progress for books copied back. A damaged file does not stop the scan.

Covers are PGM thumbnails in `<data>/covers/`, keyed by the cover bytes' MD5. They are 200 px wide, at most 320 px high, and dithered to 16 grays.

## Shelves, folders and series

The continue-reading card shows the last book opened in Aristotle. Before the first open, it uses the most recently changed progress.

- **Books** has All, Reading, New, and Finished shelves.
- **Folders** mirrors the book directory, with subfolders before books.
- **Series** groups books by series name and order.

Lists use pages, not scrolling. Sort order is recent, title, or author. Search ignores case and diacritics, including `ł`. Every query word must match the title, an author, or the series. Sorting is not locale-specific.

## Stock files: sidecars and My Clippings.txt

Aristotle reads the Kindle's reading-position sidecars (KRDS), so positions and highlights from the stock reader carry over to Aristotle. Highlights made in Aristotle go to `My Clippings.txt` in the stock format. The format is described here in our own words, from public documentation (jhowell's KRDS notes on MobileRead, [krdsrw](https://github.com/midrare/krdsrw), `kindle_formats`) and from files on the device, for interoperability. No Amazon code is included.

### Sidecar folders [fw]

For `<name>.<ext>`, the stock reader stores files in `<name>.sdr/`.

| Format | Positions | Annotations and settings |
| --- | --- | --- |
| MOBI 7 | `.mbs` | `.mbp1` |
| KF8, AZW3 | `.azw3f` | `.azw3r` |
| KFX | `.yjf` | `.yjr` |
| PDF | `.pdt` | `.pds` |
| Topaz | `.tas` | `.tal` |

The folder can also contain `.apnx` page maps and KOReader's `metadata.<ext>.lua`. The device's two sideloaded MOBI samples had `.mbs` and `.mbp1` [dev]. Aristotle reads stock sidecars but does not write them.

### KRDS

`sidecar.rs` parses big-endian typed objects. The layout matches four device `.mbs` and `.mbp1` files [dev] and firmware readers [fw].

| Object | Contents | Source |
| --- | --- | --- |
| `lpr` | Last position and epoch milliseconds | [fw][dev] |
| `fpr` | Furthest position, time, local offset, country, and device | [fw][dev] |
| `annotation.cache.object` | Typed bookmark, highlight, note, and clip objects | [fw] |

MOBI positions begin with an uncompressed-text byte offset [dev]. AZW3 uses the same unit [fw]. Annotation layouts are not verified on device data because the samples contain none.

### What is imported

New books import sidecars during a scan. `aristotle-library-cli import-sidecars` imports them for all books. Only newer progress replaces saved progress.

- MOBI and AZW3 import `lpr`, with `.apnx` page labels when available. A fraction of at least 0.99 marks completion.
- Stock annotations retain positions, creation times, and note text. Kindle locations use byte position divided by 150, plus 1 [calibre]. The same unit in firmware 5.x is an inference.
- KOReader imports `percent_finished`, `last_xpointer` or `last_page`, and `summary.status`. The partial MD5 must match.
- `fpr`, PDF and KFX positions, font preferences, and KOReader highlights are not imported.

### My Clippings.txt [fw]

The stock format is UTF-8 with CRLF lines and a BOM on creation:

```text
{title} ({author})
- {label} | Added on {date FULL} {time MEDIUM}

{text}
==========
```

`aristotle-library-cli clippings FILE --import` matches folded titles and uses authors to break ties. Repeated imports add nothing. Clippings do not set reading progress. Polish label recognition is unverified.

## Export

New highlights append to `My Clippings.txt` with stock en_GB labels, regardless of UI language. Existing entries are skipped. Notes use a preceding **Your Note** entry. Bookmarks are not exported, and deleted highlights remain in the file. The `clippings` preference controls this output.

Markdown notes in `/mnt/us/aristotle-notes/` are rewritten per book. They contain Obsidian front matter, quotes, chapters, locations, dates, and notes. Labels follow the UI language.

## CLI

`aristotle-library-cli` provides `scan`, `list`, `show`, `clippings`, `import-sidecars`, and `thumb`. Run it without arguments for usage and filters. The app uses `--library` for index data; the library CLI uses `--data`.

## Performance

Host release build, 31 books and 29 covers [host]:

| Run | Time or size |
| --- | --- |
| Cold scan with covers | 200 to 340 ms |
| Cold scan without covers | 1.8 ms |
| Unchanged rescan | 0.1 ms |
| Index | 16.8 KB JSON |
| Thumbnails | 1.7 MB |

These are host results, not PW4 timings.

## Network: progress sync and catalogs

Both features are off until configured in **Settings > Network**. Worker threads connect only to configured servers and catalogs. The device's host allowlist applies to requests and redirects ([Network policy](../device/README.md#network)).

Saving a server adds a tagged entry to `net-allow` through `aristotlectl net allow add`. Removing it deletes that entry, not untagged entries. Code lives in `crates/aristotle-net` and the app's `net.rs`, `sync.rs`, and `catalogs.rs`.

### Settings files

There is no on-screen keyboard. Edit `sync.json` and `catalogs.json` in the library data directory. Tapping an editable row names the file.

`sync.json`:

```json
{"server":"http://192.168.1.10:17200","username":"anna","password":"example","auto":true}
```

`catalogs.json`:

```json
{"catalogs":[{"name":"calibre","url":"http://192.168.1.10:8080/opds","username":"anna","password":"example"}]}
```

On import, the app removes password fields from these userstore files. Credentials go to `/var/local/aristotle/accounts.json`, mode 0600, in a root-owned directory with mode 000. The app rejects credential directories with group or other permissions. This protects against the stock framework's recursive permission changes, observed on the device [dev]. `ARISTOTLE_ACCOUNTS_DIR` overrides the directory for development. Agent removal deletes it.

Sync stores `md5(password)`, which still authenticates to that server. Catalogs retain the password for Basic and Digest authentication. Catalog imports match by name and keep a stored password when no replacement is supplied.

### Progress sync (KOSync)

Aristotle uses KOReader's protocol with compatible servers [KOReader]. The document ID is the partial MD5 without a duplicate suffix. A random device ID is saved once.

| Format | Position sent | Position received |
| --- | --- | --- |
| EPUB | Chapter start, `/body/DocFragment[N]/body` | Chapter, refined by percentage when possible |
| PDF, CBZ | 1-based page | Page |
| MOBI, AZW3, TXT | No position | Percentage |

`Fragments` maps EPUB chapter numbers between Plato and KOReader. Element paths after `DocFragment[N]` are not resolved.

Automatic sync fetches progress on open with a 5-second timeout. A newer position on another page needs confirmation before a jump. Closing a book sends changed progress. Errors appear on the sync screen without interrupting reading. Failed pushes are not queued.

### Catalogs (OPDS 1.2)

Catalogs support Atom navigation, acquisition, paging, and OpenSearch or `{searchTerms}` search. EPUB, AZW3, MOBI, PDF, CBZ, and TXT downloads are offered, EPUB first. Shop, loan, and indirect links are skipped. Feeds are limited to 2 MiB and show no covers.

Downloads go to `documents/<catalog name>/<title> - <author>.<ext>`. Each is limited to 200 MiB and leaves at least 20 MiB free. A hidden `.part` file is synced and renamed on success. Failure or cancellation removes it. Existing files are not replaced. The library scans after download.

HTTPS uses the `tls` feature, enabled in release bundles by `APP_FEATURES` in `tools/build-render.sh`. Certificate-time errors ask the user to check the date and time.

## Known limitations

- Stock annotations and KF8 sidecars have no device-data verification.
- PDF thumbnails and GIF or WebP cover decoding are unsupported.
- KOReader imports progress and status, not highlights.
