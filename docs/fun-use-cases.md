# Fun use cases

Aristotle reads whatever lands in `/mnt/us/documents`, and the USB network gives you SSH to the device. So anything on your computer that writes an EPUB can feed the Kindle without Amazon. None of this is part of Aristotle itself; these are just things that are nice to have.

## A daily newspaper

Turn Hacker News and your RSS feeds into one clean EPUB a day and copy it into `documents/Gazeta/`. The library shows it like any folder.

- `aristotle-press`, a small separate tool, does this: reader-mode extraction, photos redrawn as minimal outline sketches (or described in one line by an LLM), the top HN comments, and a sync script that keeps the last few issues.
- Calibre's **Fetch news** does much the same with its built-in recipes. Save the result as EPUB and copy it over.

## Send a web page to the Kindle

Any article becomes an EPUB with [percollate](https://github.com/danburzo/percollate) or pandoc, and one `scp` puts it in the library:

```sh
percollate epub https://example.com/long-read -o long-read.epub
scp -F devloop/host/ssh_config long-read.epub kindle:/mnt/us/documents/Inbox/
```

## Highlights in your notes

Highlights go to `My Clippings.txt` in the stock Kindle format, so existing tools (Obsidian plugins, Readwise importers, your own scripts) keep working. Aristotle also writes one Markdown file per book to `/mnt/us/aristotle-notes/`.

## Dictionaries for any language

Drop StarDict dictionaries (for example the Wiktionary exports for Ancient Greek or Latin) into the dictionary folder, and holding a word looks it up offline. Greek lookups ignore accents and breathings.
