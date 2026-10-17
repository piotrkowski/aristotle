# Back up and update Aristotle

Quit Aristotle before copying its data or replacing the app. A running app can save over restored files. These procedures concern the app and reading data, not recovery from an unbootable Kindle. No full PW4 recovery path is verified.

Paths below are relative to the Kindle's USB drive unless they start with `/`. Enable hidden files in your file manager to see `.aristotle/` and book sidecars.

## Back up reading data

1. Return to Aristotle's library and hold **Library** for one second to quit. Wait for the stock interface.
2. Connect the Kindle in [USB drive mode](first-book.md#copy-the-book).
3. Create a dated backup folder on the computer.
4. Copy `.aristotle/`, `documents/`, and `aristotle-notes/` into it, if present.
5. Copy your `fonts/` and `dictionaries/` folders if you cannot replace them from another source. Include `koreader/data/dict/` if you keep dictionaries there.
6. Keep the release zip you installed and note the version from `aristotle/SOURCE.txt`.

Copy `documents/` with its hidden files and `.sdr` folders. It includes the books, stock sidecars, and `My Clippings.txt`. Markdown exports alone are not a complete backup of bookmarks, preferences, or reading positions.

The USB backup does not include `/var/local/aristotle/accounts.json`. Agent removal deletes that file. Keep your sync and catalog credentials separately so you can [import them again](network.md). The USB import files have their passwords removed after import.

## Know which files contain your state

| Path | Contents | Can be rebuilt without losing reading state? |
| --- | --- | --- |
| `.aristotle/library.json` | Index, progress, status, per-book settings, and annotations | No. A rescan rebuilds metadata, not Aristotle's annotations or progress. |
| `.aristotle/reader.json` | Global preferences | No |
| `.aristotle/reading.json` | Reading-speed estimates | No |
| `.aristotle/dictionaries.json` | Preferred dictionary per language | No |
| `.aristotle/covers/` | Generated cover thumbnails | Yes |
| `.aristotle/pages/` | Pagination cache | Yes |
| `.aristotle/snapshots/` | Saved rendered pages | Yes |
| `.aristotle/epub-cache/` | Converted MOBI books and position maps | Yes, from the original books |
| `.aristotle/dict-index/` | Dictionary lookup indexes | Yes, from the dictionary files |
| `.aristotle/fonts.json`, `.aristotle/reading-fonts/` | Discovered fonts and cached font copies | Yes, if the original fonts remain in `fonts/` |
| `aristotle-notes/` | Markdown annotation exports | Not a complete state backup |

Do not delete the whole `.aristotle/` directory to clear a cache. Move only the relevant cache aside while the app is stopped.

## Update the app

1. Make a backup and quit the app as described above.
2. Read the new release notes for compatibility or migration changes.
3. Verify the new zip with its published `SHA256SUMS`, or use the [attestation command](install.md#install).
4. Extract the zip to the Kindle drive root, replacing `aristotle/` and `extensions/aristotle/` files. Keep `.aristotle/` and your books.
5. Eject the drive.
6. If you use autostart, choose **KUAL > Aristotle > Start Aristotle at boot: on** again. This refreshes the internal app copy.
7. Start Aristotle and check the version in **Settings > About > Software**.
8. Open a recent book and check its position and highlights.

Replacing the release files does not reinstall the agent's private scripts or upstart jobs. If the release changes the agent, follow its release notes and select **Install agent** to apply that update. Agent installation writes the three jobs under `/etc/upstart/`.

Boot startup uses the internal copy made in step 6. Starting from KUAL uses the USB copy. Updating only the USB copy leaves the old version running at the next boot.

## Restore a backup

1. Quit Aristotle and connect the Kindle in USB drive mode.
2. Copy the current `.aristotle/` and book files to a separate folder on the computer before replacing anything.
3. Restore the backed-up `.aristotle/`, `documents/`, and `aristotle-notes/` to their original paths. Use the backup as the replacement state, not a mixture of two `.aristotle/` directories.
4. Restore any fonts or dictionaries the backup depends on.
5. Eject the drive and start the matching app version, or a version documented to accept that state.
6. Check a known reading position and highlight.

Do not restore `/var/local/aristotle/` wholesale. Install the agent through KUAL and reconfigure accounts instead.

## Return to an earlier release

Downgrades are not guaranteed. An older build rejects an index with a newer schema version. Keeping books alone will not preserve all reading state.

1. Save the current app version and reading data separately.
2. Quit Aristotle and extract the earlier release zip to the drive root.
3. Restore the `.aristotle/` backup made with that release, if the older app cannot read the current state.
4. Eject the drive and refresh the internal app copy if autostart is enabled.
5. Start the app and check the version and restored state.

Restoring an older backup loses changes made since that backup. Keep the newer backup until you have verified the result. Do not edit the index's version number to bypass a compatibility error.
