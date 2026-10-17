// Device screenshots and clickable areas, in panel pixels.
window.ARISTOTLE_DEMO = {
  "screen": { "width": 1072, "height": 1448 },
  "start": "library",
  "frames": {
    "library": {
      "image": "demo/frames/library.png",
      "alt": "The library: Nicomachean Ethics, Book I, by Aristotle as the book to continue, at 33 %, above a list of four books with the shelves All, Reading, New and Finished.",
      "hint": "Tap Continue reading to start.",
      "hotspots": [
        { "id": "continue", "label": "Open Nicomachean Ethics", "rect": [60, 215, 950, 250], "to": "greek-page", "kind": "flash" },
        { "id": "settings", "label": "Settings", "rect": [880, 60, 136, 120], "to": "settings", "kind": "flash" }
      ]
    },
    "greek-page": {
      "image": "demo/frames/greek-page.png",
      "alt": "The first page of Book I in Bywater's Greek text, Βιβλίον Α, justified, with the chapter and progress in the footer.",
      "hint": "Tap ἀγαθοῦ to look it up.",
      "hotspots": [
        { "id": "next", "label": "Next page", "rect": [321, 173, 751, 1275], "to": "english-page", "kind": "turn", "key": "ArrowRight" },
        { "id": "word", "label": "Look up ἀγαθοῦ", "rect": [352, 318, 172, 80], "to": "greek-popup", "kind": "turn" }
      ]
    },
    "greek-popup": {
      "image": "demo/frames/greek-popup.png",
      "alt": "The dictionary popup for ἀγαθοῦ: its base form ἀγαθός, an adjective, good; brave; noble; moral; gentle, from the grc-en dictionary, with a tab for grc-pl and the buttons Highlight, More and Close.",
      "hint": "Tap Close to keep reading.",
      "hotspots": [
        { "id": "page", "label": "Close the popup", "rect": [0, 0, 1072, 760], "to": "greek-page", "kind": "turn" },
        { "id": "close", "label": "Close", "rect": [458, 1318, 140, 112], "to": "greek-page", "kind": "turn", "key": "Escape" }
      ]
    },
    "english-page": {
      "image": "demo/frames/english-page.png",
      "alt": "Book I, Chapter I in Chase's English translation, justified and hyphenated: Every art, and every science reduced to a teachable form, and in like manner every action and moral choice, aims, it is thought, at some good.",
      "hint": "Tap the top edge for the menu.",
      "hotspots": [
        { "id": "menu", "label": "Open the menu", "rect": [0, 0, 1072, 173], "to": "menu", "kind": "overlay" },
        { "id": "prev", "label": "Previous page", "rect": [0, 173, 321, 1275], "to": "greek-page", "kind": "turn", "key": "ArrowLeft" }
      ]
    },
    "menu": {
      "image": "demo/frames/menu.png",
      "alt": "The reader menu over the English page: a top bar with Library, the title, Contents, Highlights and Go to, and a bottom sheet with the chapter and page, size, spacing, margins, light and a Night switch.",
      "hint": "Back to the library, or tap the page.",
      "hotspots": [
        { "id": "library", "label": "Back to the library", "rect": [30, 10, 280, 110], "to": "library", "kind": "flash" },
        { "id": "close", "label": "Close the menu", "rect": [0, 240, 1072, 420], "to": "english-page", "kind": "turn", "key": "Escape" }
      ]
    },
    "settings": {
      "image": "demo/frames/settings.png",
      "alt": "Settings: the frontlight at 2 of 24 with presets, a Night mode switch, covers in the library, battery and the screensaver delay.",
      "hint": "Try night mode.",
      "hotspots": [
        { "id": "back", "label": "Back to the library", "rect": [30, 50, 400, 136], "to": "library", "kind": "flash", "key": "Escape" },
        { "id": "night", "label": "Night mode", "rect": [40, 545, 992, 120], "to": "settings-night", "kind": "flash" }
      ]
    },
    "settings-night": {
      "image": "demo/frames/settings-night.png",
      "alt": "Settings in night mode, white on black, with the Night mode switch on.",
      "hint": "Go back to the library.",
      "hotspots": [
        { "id": "back", "label": "Back to the library", "rect": [30, 50, 400, 136], "to": "library-night", "kind": "flash", "key": "Escape" },
        { "id": "night", "label": "Day mode", "rect": [40, 545, 992, 120], "to": "settings", "kind": "flash" }
      ]
    },
    "library-night": {
      "image": "demo/frames/library-night.png",
      "alt": "The library in night mode, white on black.",
      "hint": "Open the book in night mode.",
      "hotspots": [
        { "id": "continue", "label": "Open Nicomachean Ethics", "rect": [60, 215, 950, 250], "to": "greek-page-night", "kind": "flash" },
        { "id": "settings", "label": "Settings", "rect": [880, 60, 136, 120], "to": "settings-night", "kind": "flash" }
      ]
    },
    "greek-page-night": {
      "image": "demo/frames/greek-page-night.png",
      "alt": "The first page of Book I in Greek, in night mode.",
      "hint": "Tap a word, or the top edge for the menu.",
      "hotspots": [
        { "id": "menu", "label": "Open the menu", "rect": [0, 0, 1072, 173], "to": "menu-night", "kind": "overlay" },
        { "id": "next", "label": "Next page", "rect": [321, 173, 751, 1275], "to": "english-page-night", "kind": "turn", "key": "ArrowRight" },
        { "id": "word", "label": "Look up ἀγαθοῦ", "rect": [352, 318, 172, 80], "to": "greek-popup-night", "kind": "turn" }
      ]
    },
    "greek-popup-night": {
      "image": "demo/frames/greek-popup-night.png",
      "alt": "The dictionary popup for ἀγαθοῦ in night mode.",
      "hint": "Tap Close to keep reading.",
      "hotspots": [
        { "id": "page", "label": "Close the popup", "rect": [0, 0, 1072, 760], "to": "greek-page-night", "kind": "turn" },
        { "id": "close", "label": "Close", "rect": [458, 1318, 140, 112], "to": "greek-page-night", "kind": "turn", "key": "Escape" }
      ]
    },
    "english-page-night": {
      "image": "demo/frames/english-page-night.png",
      "alt": "Book I, Chapter I in English, in night mode.",
      "hint": "Tap the left side to go back.",
      "hotspots": [
        { "id": "prev", "label": "Previous page", "rect": [0, 173, 321, 1275], "to": "greek-page-night", "kind": "turn", "key": "ArrowLeft" }
      ]
    },
    "menu-night": {
      "image": "demo/frames/menu-night.png",
      "alt": "The reader menu over the Greek page in night mode, with the Night switch on.",
      "hint": "Tap Library to go back.",
      "hotspots": [
        { "id": "library", "label": "Back to the library", "rect": [30, 10, 280, 110], "to": "library-night", "kind": "flash" },
        { "id": "close", "label": "Close the menu", "rect": [0, 240, 1072, 420], "to": "greek-page-night", "kind": "turn", "key": "Escape" }
      ]
    }
  }
};
