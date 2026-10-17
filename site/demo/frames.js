// The demo's frame set. Everything below the first line is plain JSON, so a
// script can rewrite it; it is wrapped in a script so the site also works from
// file://, where browsers refuse to fetch a .json file. To swap in device
// captures or another language, change this file and the images it names.
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
// tour: the guided path. A step is reached when its frame is shown after the
//   previous step's. "target" marks the hotspot to suggest; "action" adds a
//   button below the device.
window.ARISTOTLE_DEMO = {
  "screen": { "width": 1072, "height": 1448 },
  "start": "library",
  "note": "Frames are host renders of the current interface, pixel for pixel; the interface is in Polish for now. The dictionary step uses a short test text. Books shown are in the public domain.",
  "frames": {
    "library": {
      "image": "demo/frames/library.png",
      "alt": "The library: Lalka by Bolesław Prus as the book to continue, at 43 %, above a list of four books with progress bars.",
      "hotspots": [
        { "id": "continue", "label": "Open Lalka", "rect": [71, 187, 930, 294], "to": "page-12", "kind": "flash" },
        { "id": "settings", "label": "Settings", "rect": [880, 50, 136, 136], "to": "settings", "kind": "flash" }
      ]
    },
    "library-night": {
      "image": "demo/frames/library-night.png",
      "alt": "The library in night mode, white on black.",
      "hotspots": [
        { "id": "continue", "label": "Open Lalka", "rect": [71, 187, 930, 294], "to": "page-12-night", "kind": "flash" },
        { "id": "settings", "label": "Settings", "rect": [880, 50, 136, 136], "to": "settings-night", "kind": "flash" }
      ]
    },
    "settings": {
      "image": "demo/frames/settings.png",
      "alt": "Settings: frontlight in 24 steps with presets, night mode, covers in the library, battery and screensaver delay.",
      "hotspots": [
        { "id": "back", "label": "Back to the library", "rect": [30, 50, 400, 136], "to": "library", "kind": "flash", "key": "Escape" }
      ]
    },
    "settings-night": {
      "image": "demo/frames/settings-night.png",
      "alt": "Settings in night mode.",
      "hotspots": [
        { "id": "back", "label": "Back to the library", "rect": [30, 50, 400, 136], "to": "library-night", "kind": "flash", "key": "Escape" }
      ]
    },
    "page-12": {
      "image": "demo/frames/page-12.png",
      "alt": "Page 12 of 684 of Lalka: the opening of chapter one, justified and hyphenated, with the chapter and minutes left in the footer.",
      "hotspots": [
        { "id": "menu", "label": "Open the menu", "rect": [0, 0, 1072, 173], "to": "menu", "kind": "overlay" },
        { "id": "next", "label": "Next page", "rect": [321, 173, 751, 1275], "to": "page-13", "kind": "turn", "key": "ArrowRight" }
      ]
    },
    "page-13": {
      "image": "demo/frames/page-13.png",
      "alt": "Page 13 of 684 of Lalka, full of text.",
      "hotspots": [
        { "id": "prev", "label": "Previous page", "rect": [0, 173, 321, 1275], "to": "page-12", "kind": "turn", "key": "ArrowLeft" }
      ]
    },
    "page-12-night": {
      "image": "demo/frames/page-12-night.png",
      "alt": "Page 12 of Lalka in night mode, white text on black.",
      "hotspots": [
        { "id": "menu", "label": "Open the menu", "rect": [0, 0, 1072, 173], "to": "menu-night", "kind": "overlay" },
        { "id": "next", "label": "Next page", "rect": [321, 173, 751, 1275], "to": "page-13-night", "kind": "turn", "key": "ArrowRight" }
      ]
    },
    "page-13-night": {
      "image": "demo/frames/page-13-night.png",
      "alt": "Page 13 of Lalka in night mode.",
      "hotspots": [
        { "id": "prev", "label": "Previous page", "rect": [0, 173, 321, 1275], "to": "page-12-night", "kind": "turn", "key": "ArrowLeft" }
      ]
    },
    "menu": {
      "image": "demo/frames/menu.png",
      "alt": "The reader menu over the page: a top bar with Biblioteka (library) and the title, and a bottom sheet with chapter and page, font, size, spacing, margins, light and Noc (night).",
      "hotspots": [
        { "id": "library", "label": "Back to the library", "rect": [30, 10, 300, 110], "to": "library", "kind": "flash" },
        { "id": "close", "label": "Close the menu", "rect": [0, 130, 1072, 410], "to": "page-12", "kind": "turn", "key": "Escape" },
        { "id": "night", "label": "Night mode", "rect": [40, 1305, 992, 110], "to": "menu-night", "kind": "flash" }
      ]
    },
    "menu-night": {
      "image": "demo/frames/menu-night.png",
      "alt": "The reader menu in night mode, with the Noc switch on.",
      "hotspots": [
        { "id": "library", "label": "Back to the library", "rect": [30, 10, 300, 110], "to": "library-night", "kind": "flash" },
        { "id": "close", "label": "Close the menu", "rect": [0, 130, 1072, 410], "to": "page-12-night", "kind": "turn", "key": "Escape" },
        { "id": "night", "label": "Day mode", "rect": [40, 1305, 992, 110], "to": "menu", "kind": "flash" }
      ]
    },
    "test-page-night": {
      "image": "demo/frames/test-page-night.png",
      "alt": "A test text in night mode: Rozdział pierwszy. O duszy, with Polish, Greek and Latin words and a footnote marker.",
      "hotspots": [
        { "id": "word", "label": "Hold the word dusza", "rect": [480, 270, 150, 70], "to": "test-dict-night", "kind": "turn" }
      ]
    },
    "test-dict-night": {
      "image": "demo/frames/test-dict-night.png",
      "alt": "The dictionary popup for dusza (soul) from a Polish Wiktionary dictionary, with tabs for pl-pl, la-pl and en-pl, six senses, and Zakreśl (highlight), Więcej (more) and Zamknij (close).",
      "hotspots": [
        { "id": "page", "label": "Close the popup", "rect": [0, 0, 1072, 380], "to": "test-page-night", "kind": "turn" },
        { "id": "close", "label": "Close", "rect": [450, 1320, 205, 110], "to": "test-page-night", "kind": "turn", "key": "Escape" }
      ]
    }
  },
  "tour": [
    { "frame": "library", "target": "continue", "hint": "This is the library. Tap Lalka, the book to continue, to open it where you left off." },
    { "frame": "page-12", "target": "next", "hint": "Tap the right of the page, or press <kbd>→</kbd>, to turn it. The page flashes once, as on the panel." },
    { "frame": "page-13", "target": "prev", "hint": "Tap the left third, or press <kbd>←</kbd>, to go back a page." },
    { "frame": "page-12", "target": "menu", "hint": "Tap the top of the page to open the menu. It appears without a flash." },
    { "frame": "menu", "target": "night", "hint": "Turn on Noc, night mode." },
    { "frame": "menu-night", "target": "close", "hint": "Tap the page to close the menu." },
    { "frame": "page-12-night", "hint": "Night mode stays on for every screen. Next, the dictionary, on a test text with Polish, Greek and Latin words.", "action": { "label": "Open the test text", "to": "test-page-night", "kind": "flash" } },
    { "frame": "test-page-night", "target": "word", "hint": "Hold a word to look it up. Tap the marked word, dusza." },
    { "frame": "test-dict-night", "target": "close", "hint": "The definition comes from an offline dictionary. Tap Zamknij to go back to the page." },
    { "frame": "test-page-night", "hint": "That is the tour. Everything else in the demo still responds to taps.", "action": { "label": "Start again", "to": "library", "kind": "flash" } }
  ]
};
