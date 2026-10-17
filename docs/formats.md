# Format support

The library detects books by content. The renderer also uses file extensions to choose an engine, so a renamed file can be indexed but still fail to open. The tables describe the default release bundle, not every format supported by upstream Plato or MuPDF.

## Default release bundle

| Format | Library metadata | Cover | Reading | Text layout |
| --- | --- | --- | --- | --- |
| EPUB | OPF title, authors, and series | Embedded cover | Yes | Plato, with custom fonts and hyphenation |
| DRM-free MOBI 7 | Record 0 and EXTH | Embedded cover | Yes | Converted to EPUB when supported, otherwise MuPDF fallback |
| DRM-free AZW3, KF8-only | Record 0 and EXTH | Embedded cover | MuPDF fallback | Not converted by Aristotle's MOBI 7 converter |
| PalmDOC | Database name | None | MOBI path | Depends on conversion or fallback |
| PDF | Info metadata and page count | None | Yes | Fixed pages scaled to fit |
| TXT | File and folder names | None | Yes | MuPDF reflow, without the selected custom font |
| CBZ | ComicInfo metadata and page count | First image | No | CBZ reader disabled in the default MuPDF build |

MOBI conversion does not support HUFF/CDIC compression or KF8-only content. Encrypted MOBI can appear in the index with its DRM flag, but cannot be read. Aristotle does not remove DRM.

KFX, CBR, CB7, FB2, DjVu, and Topaz are unsupported by the library. A custom `MUPDF_FORMATS=full` build enables more renderer formats, including CBZ, but does not add library detection for every format. See [Size budget](reader.md#size-budget).

## Reading interactions

| Capability | Scope |
| --- | --- |
| Custom fonts and Aristotle's full text layout | EPUB and successfully converted MOBI |
| Word selection, dictionary lookup, and highlights | Text exposed by the document engine. An image-only PDF has no selectable words. |
| Footnote popup | Recognized EPUB note links and converted content that retains them |
| Table of contents | Document-provided entries. Converted MOBI has no table of contents. |
| Saved reading position | EPUB, MOBI, AZW3, PDF, and TXT, using format-specific positions |
| Clippings export | New highlights and attached notes. Bookmarks are not exported. |
| Markdown export | Per-book annotation export. Not a replacement for `library.json`. |

Stock annotation imports and KF8 sidecars have no device-data verification. KOReader import preserves progress and status, not highlights. See [Library imports](library.md#stock-files-sidecars-and-my-clippingstxt).

## Progress sync

| Format | Position sent to KOSync | Position received |
| --- | --- | --- |
| EPUB | Chapter start | Chapter, refined by percentage when possible |
| PDF | 1-based page | Page |
| MOBI, AZW3, TXT | Percentage, without a position string | Percentage |
| CBZ in a custom reader build | 1-based page | Page |

Sync uses a partial file MD5 as the document ID. Different editions or reconverted files need not share that ID. EPUB element paths after the chapter are not resolved. [Network setup](network.md) covers configuration and a first sync check.

## Catalog downloads

OPDS acquisition supports EPUB, AZW3, MOBI, PDF, CBZ, and TXT links, with EPUB offered first. Being offered for download does not mean a format opens in the default reader. Shop, loan, and indirect acquisition links are skipped.

Implementation references: [library detection](library.md#what-it-indexes), [reader engines and positions](reader.md#formats-and-positions), and [OPDS behavior](library.md#catalogs-opds-12).
