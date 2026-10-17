# Documentation

Aristotle runs on a jailbroken Kindle Paperwhite 4 on firmware 5.13.x. Start with the installation guide if you have not installed it yet.

## Use Aristotle

| Task | Guide |
| --- | --- |
| Install or remove Aristotle and its optional agent | [Install](install.md) |
| Copy a book, read it, and save a highlight | [Your first book](first-book.md) |
| Add an offline dictionary | [Install dictionaries](dictionaries.md) |
| Connect progress sync or an OPDS catalog | [Set up network features](network.md) |
| Back up data, update, or restore a previous version | [Back up and update](maintenance.md) |
| Diagnose a problem or collect logs | [Troubleshooting](troubleshooting.md) |
| Check what a book format supports | [Format support](formats.md) |

The default screensaver does not suspend the CPU, and the power button has no effect while Aristotle runs. Battery use has not been verified. Quit to the stock interface when you finish reading. See [Power and sleep](reader.md#the-app) for the experimental alternative.

## Understand the implementation

| Topic | Reference |
| --- | --- |
| Reading state, rebuildable caches, and network import paths | [Data and file locations](data.md) |
| Crates, processes, and extension points | [Architecture](architecture.md) |
| Rendering, reading interactions, threads, and preferences | [Reader](reader.md) |
| Index, metadata, import, export, and network protocols | [Library](library.md) |
| Widgets, layout, translations, and panel updates | [UI framework](ui-framework.md) |
| KUAL, agent, USB networking, boot startup, and network policy | [Device-side code](../device/README.md) |
| Hardware and firmware evidence | [Paperwhite 4 reference](pw4-reference.md) |
| Measurements and how to repeat them | [Benchmarks](benchmarks.md) |

## Develop and contribute

- [Build and check the project](../.github/CONTRIBUTING.md#building).
- [Set up host access over USB](../device/README.md#host-setup).
- [Report a security issue privately](../.github/SECURITY.md).
- [View screenshots](screenshots.md) or the [interactive demo](https://piotrkowski.github.io/aristotle/).
