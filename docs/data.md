# Data and file locations

These are the default device paths. Paths without a leading `/` are relative to the Kindle's USB drive, mounted at `/mnt/us` on the device. `.aristotle/` is hidden in many file managers.

## Books and user files

| Path on the USB drive | Contents |
| --- | --- |
| `documents/` | Book files and stock `.sdr` sidecars |
| `documents/My Clippings.txt` | Existing clippings and new highlights appended by Aristotle when enabled |
| `aristotle-notes/` | Per-book Markdown exports of Aristotle highlights and attached notes, not bookmarks |
| `fonts/` | User-supplied TTF and OTF fonts |
| `dictionaries/`, `koreader/data/dict/` | User-supplied StarDict dictionaries |
| `aristotle/` | Installed app, runtime assets under `data/`, licenses, and `SOURCE.txt` |
| `extensions/aristotle/` | KUAL menu and agent installer |

Exports do not replace the saved reading state. Stock sidecars are read, not rewritten by Aristotle. The [library reference](library.md#stock-files-sidecars-and-my-clippingstxt) describes imports and exports.

## Reading state

| Path on the USB drive | Contents |
| --- | --- |
| `.aristotle/library.json` | Book metadata, progress, status, per-book settings, and annotations |
| `.aristotle/reader.json` | Global preferences |
| `.aristotle/reading.json` | Reading-speed estimates |
| `.aristotle/dictionaries.json` | Preferred dictionary per language |

Rescanning books can rebuild metadata, but not all progress and annotations created in Aristotle. A [reading-data backup](maintenance.md#back-up-reading-data) includes these state files.

## Rebuildable caches

| Path under `.aristotle/` | Generated from |
| --- | --- |
| `covers/` | Embedded book covers |
| `pages/` | Book pagination |
| `snapshots/` | Rendered pages |
| `epub-cache/` | Original MOBI books, including converted EPUBs and position maps |
| `dict-index/` | Original dictionary files |
| `fonts.json`, `reading-fonts/` | Font files in `fonts/` |

These caches can be regenerated if their source files remain available. An unchanged library scan does not recreate missing cover thumbnails. A backup containing `covers/` preserves those thumbnails without a rebuild. The whole `.aristotle/` directory is not a cache: it also contains reading state.

## Network settings and agent state

| Device path | Contents |
| --- | --- |
| `/mnt/us/aristotle/sync.json`, `/mnt/us/aristotle/catalogs.json` | Account import files for the manual KUAL app |
| `/var/local/aristotle/accounts.json` | Saved sync key, catalog credentials, device ID, and import-file signatures |
| `/var/local/aristotle/app/` | Internal app copy used by autostart |
| `/var/local/aristotle/net-mode`, `/var/local/aristotle/net-allow` | Agent network policy |

The import directory is the directory containing the running executable. A boot launch therefore looks in `/var/local/aristotle/app/`, not `/mnt/us/aristotle/`. The network screens display the actual import paths. Password fields are removed from successfully imported JSON files. Saved credentials are private and are not part of a USB-drive backup.

`--library DIR` or `ARISTOTLE_DATA` changes the reading-state and cache directory. `--data DIR` selects renderer assets, not reading state or network imports. `--root DIR` changes the book root and export paths. `ARISTOTLE_ACCOUNTS_DIR` changes the private accounts directory for development.

Agent removal deletes the private credentials and internal app copy, but leaves books, exports, and `.aristotle/` on the USB drive. See [Device-side code](../device/README.md) for the agent's other files and [Log locations](troubleshooting.md#find-the-right-log) for diagnostic output.
