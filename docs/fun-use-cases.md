# Fun use cases

Aristotle reads whatever is in `/mnt/us/documents`, and the USB network gives you SSH to the device. Anything on your computer that writes an EPUB can feed the Kindle without Amazon. None of this is part of Aristotle.

## A daily newspaper

Turn Hacker News and your RSS feeds into one EPUB a day and copy it into `documents/Gazeta/`, which the library shows as a folder. A short script can extract the articles, add the top Hacker News comments and keep the last few issues on the device. Calibre's **Fetch news** builds a similar EPUB from its built-in recipes.

## Send a web page to the Kindle

[percollate](https://github.com/danburzo/percollate) or pandoc turns an article into an EPUB, and `scp` puts it in the library:

```sh
percollate epub https://example.com/long-read -o long-read.epub
scp -F devloop/host/ssh_config long-read.epub kindle:/mnt/us/documents/Inbox/
```

## Highlights in your notes

Highlights go to `My Clippings.txt` in the stock format, so tools that import it keep working. Aristotle also writes a Markdown file per book to `/mnt/us/aristotle-notes/`. Turning off "Save highlights to My Clippings.txt" in the settings leaves only the Markdown notes.

## Dictionaries for any language

Put StarDict dictionaries, such as the Wiktionary exports for Ancient Greek or Latin, into the dictionary folder. Hold a word to look it up offline. Greek lookups ignore accents and breathings.
