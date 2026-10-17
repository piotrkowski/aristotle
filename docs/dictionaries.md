# Install offline dictionaries

Aristotle reads user-supplied StarDict dictionaries. Kindle Store dictionaries are not supported. No network connection is needed for lookups.

## Copy a complete dictionary

1. Obtain a StarDict dictionary that you may legally use.
2. Extract its archive on the computer.
3. Check that the dictionary has an `.ifo` file, an uncompressed `.idx` file, and either a `.dict` or `.dict.dz` file. A `.syn` file is optional.
4. Quit Aristotle and connect the Kindle in [USB drive mode](first-book.md#copy-the-book).
5. Copy the complete dictionary into `dictionaries/` at the drive root, or into one subfolder below it.

For example, a Greek-to-Polish dictionary can have this layout:

```text
dictionaries/
    grc-pl/
        grc-pl.ifo
        grc-pl.idx
        grc-pl.dict.dz
        grc-pl.syn
```

All files in the set must share the same name before the extension. If the source uses `.idx.gz`, decompress that file to `.idx`. Keep `.dict.dz` compressed. Do not copy only the `.ifo`, or leave the set inside a ZIP archive.

Names such as `grc-pl` identify source and target languages. If you change the name, change every file in the set to match. Aristotle also searches `koreader/data/dict/`, so dictionaries already installed there can be reused. Discovery includes one subfolder level, not arbitrary nesting.

## Check a lookup

1. Eject the drive and start Aristotle.
2. Open a book in the dictionary's source language.
3. Hold a word for about half a second.
4. If several dictionaries are available, choose the one you want in the popup.

The first start builds lookup indexes in `.aristotle/dict-index/`. Lookups may be incomplete while indexing runs. The selected dictionary is remembered for the word's language.

Greek text selects Ancient Greek dictionaries. Other text uses the book's language. Inflected forms can match listed forms or suffix rules. Guessed matches are labeled.

If no dictionary appears, check the complete file set, folder depth, and language names. For parse errors, collect the [app log](troubleshooting.md#find-the-right-log).

## Build large indexes on the host

Dictionary indexing reached 66 MB on the device [dev]. To build indexes on a Linux computer instead, first [build the host app](../.github/CONTRIBUTING.md#building). Then run from the repository:

```sh
. tools/env.sh
"$ARISTOTLE_CACHE/render/host/aristotle" \
	--index-dicts \
	--dict "$ARISTOTLE_CACHE/dictionaries" \
	--library "$ARISTOTLE_CACHE/dictionary-state"
```

Put the dictionary sets in `$ARISTOTLE_CACHE/dictionaries` before running the command. After it finishes, quit the device app and copy the contents of `$ARISTOTLE_CACHE/dictionary-state/dict-index/` into `.aristotle/dict-index/` on the Kindle drive. Keep the dictionary names unchanged between host and device.

See the [dictionary implementation](reader.md#dictionary) for lookup and indexing details.
