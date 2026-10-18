// Device captures and clickable areas, in panel pixels; samples are not consecutive pages.
(function () {
  "use strict";

  function spot(id, label, rect, to, key) {
    return { id: id, label: label, rect: rect, to: to, key: key };
  }

  var library = spot("library", "Back to the library", [58, 24, 232, 76], "library");
  var menu = spot("menu", "Open the reader menu", [0, 0, 1072, 173], "menu");
  var scenes = {
    library: {
      alt: "Aristotle's library, with Nicomachean Ethics as the book to continue, a book list, shelves and folders.",
      hint: "Open a book to try the reader.", action: "continue",
      spots: [
        spot("continue", "Continue reading", [58, 208, 956, 260], "resume"),
        spot("settings", "Open settings", [908, 78, 80, 80], "settings"),
        spot("folders", "Browse folders", [202, 519, 148, 68], "folders")
      ]
    },
    folders: {
      alt: "The library's Folders tab, with Aristotle, Marcus Aurelius and Plato folders.",
      hint: "Open Aristotle's folder.", action: "folder",
      spots: [
        spot("books", "Back to books", [58, 519, 128, 68], "library", "Escape"),
        spot("folder", "Open the Aristotle folder", [58, 617, 956, 115], "folder"),
        spot("continue", "Continue reading", [58, 208, 956, 260], "resume"),
        spot("settings", "Open settings", [908, 78, 80, 80], "settings")
      ]
    },
    folder: {
      alt: "The Aristotle folder, with four books including Nicomachean Ethics, Book I.",
      hint: "Open Nicomachean Ethics from its folder.", action: "book",
      spots: [
        spot("back", "Back to folders", [58, 84, 78, 68], "folders", "Escape"),
        spot("book", "Open Nicomachean Ethics", [58, 199, 956, 128], "resume")
      ]
    },
    "greek-page": {
      alt: "The opening of Book I of Nicomachean Ethics, in Bywater's Greek text.",
      hint: "Look up the underlined Greek word.", action: "word", sample: "greek-page",
      spots: [
        Object.assign({}, menu, { to: "greek-menu" }),
        spot("word", "Look up ἀγαθοῦ", [352, 318, 172, 80], "greek-popup")
      ],
      cue: [365, 331, 150, 55]
    },
    "english-page": {
      alt: "The opening of Book I, Chapter I, in Chase's English translation of Nicomachean Ethics.",
      hint: "Look up “science” in the dictionary.", action: "word", sample: "english-page",
      spots: [menu, spot("word", "Look up science", [475, 275, 150, 75], "english-popup")],
      cue: [478, 285, 145, 46]
    },
    "greek-popup": {
      alt: "The dictionary definition of ἀγαθοῦ, with its base form ἀγαθός and English meanings.",
      hint: "Close the definition to keep reading.", action: "close", sample: "greek-page",
      spots: [
        spot("page", "Dismiss the Greek definition", [0, 0, 1072, 760], "greek-page"),
        spot("highlight", "Highlight the Greek word", [65, 1318, 232, 118], "greek-highlighted"),
        spot("close", "Close the definition", [460, 1318, 154, 118], "greek-page", "Escape")
      ]
    },
    "english-popup": {
      alt: "An offline English-Polish dictionary defining science as nauka and wiedza.",
      hint: "Close the definition to keep reading.", action: "close", sample: "english-page",
      spots: [
        spot("page", "Dismiss the English definition", [0, 0, 1072, 610], "english-page"),
        spot("close", "Close the definition", [460, 1318, 154, 118], "english-page", "Escape")
      ]
    },
    "greek-highlighted": {
      alt: "The Greek page with ἀγαθοῦ highlighted.",
      hint: "Open the reader menu to try night mode.", action: "menu", sample: "greek-page",
      spots: [Object.assign({}, menu, { to: "greek-menu" })]
    },
    menu: {
      alt: "The reader menu over the English sample, with text size, spacing, margins, light and a Night switch.",
      hint: "Switch on night mode.", action: "night", sample: "english-page",
      spots: [
        library,
        spot("close", "Close the reader menu", [0, 240, 1072, 410], "resume", "Escape"),
        spot("size", "Try larger text", [678, 854, 56, 56], "menu-size"),
        spot("night", "Switch to night mode", [886, 1320, 126, 82], "toggle-night")
      ]
    },
    "greek-menu": {
      alt: "The reader menu over the Greek sample, with text size, spacing, margins, light and a Night switch.",
      hint: "Switch on night mode.", action: "night", sample: "greek-page",
      spots: [
        library,
        spot("close", "Close the reader menu", [0, 240, 1072, 420], "resume", "Escape"),
        spot("night", "Switch to night mode", [886, 1320, 126, 82], "toggle-night")
      ]
    },
    "menu-size": {
      alt: "The English reader menu with text two steps larger.",
      hint: "Try the original text size again.", action: "size", sample: "english-page",
      spots: [
        library,
        spot("size", "Restore the original text size", [533, 854, 56, 56], "menu", "Escape"),
        spot("night", "Switch to night mode", [886, 1320, 126, 82], "toggle-night")
      ]
    },
    settings: {
      alt: "Settings for frontlight, night mode, library covers, battery and screensaver.",
      hint: "Switch on night mode.", action: "night",
      spots: [
        spot("back", "Back to the library", [58, 84, 78, 68], "library", "Escape"),
        spot("night", "Switch to night mode", [58, 534, 956, 112], "toggle-night")
      ]
    },
    "footnote-page": {
      alt: "A separate English reading sample with footnote references in Chapter I of Nicomachean Ethics.",
      hint: "Open the underlined footnote [3].", action: "note", sample: "footnote-page",
      spots: [spot("note", "Open footnote 3", [847, 934, 54, 55], "footnote")],
      cue: [857, 940, 35, 40]
    },
    footnote: {
      alt: "Footnote 3 in a popup, without leaving the reading page.",
      hint: "Close the note to return to the page.", action: "close", sample: "footnote-page",
      spots: [
        spot("page", "Dismiss the footnote", [0, 0, 1072, 925], "footnote-page"),
        spot("close", "Close the footnote", [377, 1318, 154, 118], "footnote-page", "Escape")
      ]
    }
  };

  var frames = {};
  Object.keys(scenes).forEach(function (id) {
    [false, true].forEach(function (night) {
      var scene = scenes[id];
      var suffix = night ? "-night" : "";
      frames[id + suffix] = {
        scene: id, night: night, sample: scene.sample,
        image: "demo/frames/" + id + suffix + ".png",
        alt: scene.alt + (night ? " In night mode." : ""),
        hint: night && scene.action === "night" ? "Return to the library in night mode." : scene.hint,
        action: night && scene.action === "night" ? (id === "settings" ? "back" : "library") : scene.action,
        cue: scene.cue,
        hotspots: scene.spots.map(function (item) {
          var to = item.to;
          if (to === "toggle-night") to = id + (night ? "" : "-night");
          else if (to !== "resume") to += suffix;
          return Object.assign({}, item, { to: to,
            label: item.id === "night" && night ? "Switch to day mode" : item.label });
        })
      };
    });
  });

  window.ARISTOTLE_DEMO = { screen: { width: 1072, height: 1448 }, start: "library", frames: frames };
})();
