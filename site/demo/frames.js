// The demo's frame set. Everything below the first line is plain JSON, so a
// script can rewrite it; it is wrapped in a script so the site also works from
// file://, where browsers refuse to fetch a .json file. The images are made by
// tools/screenshots/compose.sh from device captures; to swap them, change
// that manifest, this file, or both.
//
// screen: panel size in pixels; hotspot rects are [x, y, width, height] in
//   those pixels.
// frames: id -> image, alt text, hotspots. A hotspot names the frame it goes
//   to and how the panel gets there:
//   "turn"    one full-screen update without a flash (waveform mode 9):
//             page turns, closing the menu, popups;
//   "flash"   one full-screen flash (mode 8): opening a book, switching
//             screens, night mode;
//   "overlay" a partial update with no flash: the reader menu;
//   "none"    an instant swap.
//   "key" binds a keyboard key (ArrowRight, ArrowLeft, Escape).
//   Only transitions the captures show are offered: the day menu was
//   captured over the English page and the night menu over the Greek one, so
//   night mode is switched in the settings.
// tour: the guided path. A step is reached when its frame is shown after the
//   previous step's. "target" marks the hotspot to suggest; "action" adds a
//   button below the device.
window.ARISTOTLE_DEMO = {
  "screen": { "width": 1072, "height": 1448 },
  "start": "library",
  "note": "Frames are screenshots of a Kindle Paperwhite 4 running Aristotle. The book is from Project Gutenberg; the dictionary is a Wiktionary export (CC BY-SA).",
  "frames": {
    "library": {
      "image": "demo/frames/library.png",
      "alt": "The library: Nicomachean Ethics, Book I, by Aristotle as the book to continue, at 33 %, above a list of four books with the shelves All, Reading, New and Finished.",
      "hotspots": [
        { "id": "continue", "label": "Open Nicomachean Ethics", "rect": [60, 215, 950, 250], "to": "greek-page", "kind": "flash" },
        { "id": "settings", "label": "Settings", "rect": [880, 60, 136, 120], "to": "settings", "kind": "flash" }
      ]
    },
    "greek-page": {
      "image": "demo/frames/greek-page.png",
      "alt": "The first page of Book I in Bywater's Greek text, Βιβλίον Α, justified, with the chapter and progress in the footer.",
      "hotspots": [
        { "id": "next", "label": "Next page", "rect": [321, 173, 751, 1275], "to": "english-page", "kind": "turn", "key": "ArrowRight" },
        { "id": "word", "label": "Hold the word ἀγαθοῦ", "rect": [352, 318, 172, 80], "to": "greek-popup", "kind": "turn" }
      ]
    },
    "greek-popup": {
      "image": "demo/frames/greek-popup.png",
      "alt": "The dictionary popup for ἀγαθοῦ: its base form ἀγαθός, an adjective, good; brave; noble; moral; gentle, from the grc-en dictionary, with a tab for grc-pl and the buttons Highlight, More and Close.",
      "hotspots": [
        { "id": "page", "label": "Close the popup", "rect": [0, 0, 1072, 760], "to": "greek-page", "kind": "turn" },
        { "id": "close", "label": "Close", "rect": [458, 1318, 140, 112], "to": "greek-page", "kind": "turn", "key": "Escape" }
      ]
    },
    "english-page": {
      "image": "demo/frames/english-page.png",
      "alt": "Book I, Chapter I in Chase's English translation, justified and hyphenated: Every art, and every science reduced to a teachable form, and in like manner every action and moral choice, aims, it is thought, at some good.",
      "hotspots": [
        { "id": "menu", "label": "Open the menu", "rect": [0, 0, 1072, 173], "to": "menu", "kind": "overlay" },
        { "id": "prev", "label": "Previous page", "rect": [0, 173, 321, 1275], "to": "greek-page", "kind": "turn", "key": "ArrowLeft" }
      ]
    },
    "menu": {
      "image": "demo/frames/menu.png",
      "alt": "The reader menu over the English page: a top bar with Library, the title, Contents, Highlights and Go to, and a bottom sheet with the chapter and page, size, spacing, margins, light and a Night switch.",
      "hotspots": [
        { "id": "library", "label": "Back to the library", "rect": [30, 10, 280, 110], "to": "library", "kind": "flash" },
        { "id": "close", "label": "Close the menu", "rect": [0, 240, 1072, 420], "to": "english-page", "kind": "turn", "key": "Escape" }
      ]
    },
    "settings": {
      "image": "demo/frames/settings.png",
      "alt": "Settings: the frontlight at 2 of 24 with presets, a Night mode switch, covers in the library, battery and the screensaver delay.",
      "hotspots": [
        { "id": "back", "label": "Back to the library", "rect": [30, 50, 400, 136], "to": "library", "kind": "flash", "key": "Escape" },
        { "id": "night", "label": "Night mode", "rect": [40, 545, 992, 120], "to": "settings-night", "kind": "flash" }
      ]
    },
    "settings-night": {
      "image": "demo/frames/settings-night.png",
      "alt": "Settings in night mode, white on black, with the Night mode switch on.",
      "hotspots": [
        { "id": "back", "label": "Back to the library", "rect": [30, 50, 400, 136], "to": "library-night", "kind": "flash", "key": "Escape" },
        { "id": "night", "label": "Day mode", "rect": [40, 545, 992, 120], "to": "settings", "kind": "flash" }
      ]
    },
    "library-night": {
      "image": "demo/frames/library-night.png",
      "alt": "The library in night mode, white on black.",
      "hotspots": [
        { "id": "continue", "label": "Open Nicomachean Ethics", "rect": [60, 215, 950, 250], "to": "greek-page-night", "kind": "flash" },
        { "id": "settings", "label": "Settings", "rect": [880, 60, 136, 120], "to": "settings-night", "kind": "flash" }
      ]
    },
    "greek-page-night": {
      "image": "demo/frames/greek-page-night.png",
      "alt": "The first page of Book I in Greek, in night mode.",
      "hotspots": [
        { "id": "menu", "label": "Open the menu", "rect": [0, 0, 1072, 173], "to": "menu-night", "kind": "overlay" },
        { "id": "next", "label": "Next page", "rect": [321, 173, 751, 1275], "to": "english-page-night", "kind": "turn", "key": "ArrowRight" },
        { "id": "word", "label": "Hold the word ἀγαθοῦ", "rect": [352, 318, 172, 80], "to": "greek-popup-night", "kind": "turn" }
      ]
    },
    "greek-popup-night": {
      "image": "demo/frames/greek-popup-night.png",
      "alt": "The dictionary popup for ἀγαθοῦ in night mode.",
      "hotspots": [
        { "id": "page", "label": "Close the popup", "rect": [0, 0, 1072, 760], "to": "greek-page-night", "kind": "turn" },
        { "id": "close", "label": "Close", "rect": [458, 1318, 140, 112], "to": "greek-page-night", "kind": "turn", "key": "Escape" }
      ]
    },
    "english-page-night": {
      "image": "demo/frames/english-page-night.png",
      "alt": "Book I, Chapter I in English, in night mode.",
      "hotspots": [
        { "id": "prev", "label": "Previous page", "rect": [0, 173, 321, 1275], "to": "greek-page-night", "kind": "turn", "key": "ArrowLeft" }
      ]
    },
    "menu-night": {
      "image": "demo/frames/menu-night.png",
      "alt": "The reader menu over the Greek page in night mode, with the Night switch on.",
      "hotspots": [
        { "id": "library", "label": "Back to the library", "rect": [30, 10, 280, 110], "to": "library-night", "kind": "flash" },
        { "id": "close", "label": "Close the menu", "rect": [0, 240, 1072, 420], "to": "greek-page-night", "kind": "turn", "key": "Escape" }
      ]
    }
  },
  "tour": [
    { "frame": "library", "target": "continue", "hint": "This is the library. Tap the book to continue, Nicomachean Ethics, Book I, to open it where you left off." },
    { "frame": "greek-page", "target": "word", "hint": "The book opens in one flash, on Bywater's Greek text. Hold a word to look it up: tap ἀγαθοῦ." },
    { "frame": "greek-popup", "target": "close", "hint": "The definition comes from an offline Wiktionary dictionary. ἀγαθοῦ is an inflected form; the popup shows its dictionary form, ἀγαθός. Tap Close." },
    { "frame": "greek-page", "target": "next", "hint": "Tap the right of the page, or press <kbd>→</kbd>, to turn it. The page changes in one update, as on the panel." },
    { "frame": "english-page", "target": "menu", "hint": "Chase's English translation, justified and hyphenated. Tap the top of the page to open the menu. It appears without a flash." },
    { "frame": "menu", "target": "library", "hint": "The menu sets the size, spacing, margins and light. Tap Library to go back." },
    { "frame": "library", "target": "settings", "hint": "Tap the gear to open the settings." },
    { "frame": "settings", "target": "night", "hint": "Turn on Night mode." },
    { "frame": "settings-night", "target": "back", "hint": "Night mode is white on black on every screen, switched with one flash. Tap the arrow to go back." },
    { "frame": "library-night", "target": "continue", "hint": "Open the book again." },
    { "frame": "greek-page-night", "hint": "That is the tour. Everything else in the demo still responds to taps: the word, the menu, the next page.", "action": { "label": "Start again", "to": "library", "kind": "flash" } }
  ]
};
