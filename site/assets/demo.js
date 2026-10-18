(function () {
  "use strict";

  var data = window.ARISTOTLE_DEMO;
  var root = document.querySelector("[data-demo]");
  if (!data || !root) return;

  var img = root.querySelector(".device img");
  var device = root.querySelector(".device");
  var spots = root.querySelector(".hotspots");
  var flash = root.querySelector(".flash");
  var hint = root.querySelector(".demo-hint");
  var guide = root.querySelector("[data-advance]");
  var restart = root.querySelector("[data-restart]");
  var reveal = root.querySelector("[data-show-actions]");
  var enlarge = root.querySelector("[data-enlarge]");
  var samples = root.querySelector(".demo-samples");
  var feedback = root.querySelector(".demo-feedback");
  var invite = root.querySelector(".demo-invite");
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  var current = data.start;
  var lastRead = "greek-page";
  var visited = {};
  var busy = false;
  var cache = {};
  var suggested;

  function base(path) {
    return (root.getAttribute("data-root") || "") + path;
  }

  function destination(to) {
    return to === "resume" ? lastRead + (data.frames[current].night ? "-night" : "") : to;
  }

  function load(id) {
    if (!cache[id]) {
      cache[id] = new Promise(function (resolve, reject) {
        var image = new Image();
        var timer = window.setTimeout(function () { finish(new Error("Image load timed out")); }, 10000);
        function finish(error) {
          window.clearTimeout(timer);
          image.onload = image.onerror = null;
          if (error) reject(error);
          else resolve();
        }
        image.onload = function () { finish(); };
        image.onerror = function () { finish(new Error("Image failed to load")); };
        image.src = base(data.frames[id].image);
      }).catch(function (error) {
        delete cache[id];
        throw error;
      });
    }
    return cache[id];
  }

  function position(element, rect) {
    var x = (rect[0] + rect[2] / 2) / data.screen.width * 100 + "%";
    var y = (rect[1] + rect[3] / 2) / data.screen.height * 100 + "%";
    element.style.left = element.tagName === "BUTTON" ? "clamp(22px, " + x + ", calc(100% - 22px))" : x;
    element.style.top = element.tagName === "BUTTON" ? "clamp(22px, " + y + ", calc(100% - 22px))" : y;
    element.style.width = rect[2] / data.screen.width * 100 + "%";
    element.style.height = rect[3] / data.screen.height * 100 + "%";
  }

  function message(text) {
    feedback.hidden = !text;
    feedback.textContent = text;
  }

  function render(id) {
    var frame = data.frames[id];
    current = id;
    visited[frame.scene] = true;
    if (frame.scene === "greek-page" || frame.scene === "english-page" || frame.scene === "greek-highlighted") lastRead = frame.scene;
    if (invite) invite.hidden = frame.scene !== "library" || visited["greek-page"] || visited["english-page"];
    img.src = base(frame.image);
    img.alt = frame.alt;
    device.dataset.night = String(frame.night);
    message("");

    var action = frame.action;
    var text = frame.hint;
    if ((frame.scene === "greek-page" && visited["greek-popup"]) ||
        (frame.scene === "english-page" && visited["english-popup"])) {
      action = "menu";
      text = "Open the reader menu to try night mode.";
    }
    if (frame.scene === "footnote-page" && visited.footnote) {
      text = "Return to the English reading sample.";
      action = "english-sample";
    }
    if (frame.scene === "library" && frame.night) {
      action = "folders";
      text = "Explore the library's folders in night mode.";
    }
    if (frame.night && action === "menu") text = "Open the reader menu.";
    suggested = frame.hotspots.find(function (spot) { return spot.id === action; });
    if (action === "english-sample") {
      suggested = { label: "Open the English sample", to: "english-page" + (frame.night ? "-night" : "") };
    }
    hint.textContent = text;
    guide.setAttribute("aria-label", text);
    samples.hidden = !frame.sample;
    samples.querySelectorAll("button").forEach(function (button) {
      button.setAttribute("aria-pressed", String(button.dataset.sample === frame.sample));
    });
    spots.textContent = "";
    frame.hotspots.forEach(function (spot) {
      var button = document.createElement("button");
      button.type = "button";
      button.className = "hotspot";
      button.disabled = busy;
      button.setAttribute("aria-label", spot.label);
      button.title = spot.label;
      button.dataset.id = spot.id;
      position(button, spot.rect);
      button.addEventListener("click", function (event) {
        go(destination(spot.to), event.detail === 0);
      });
      spots.appendChild(button);
      var marker = document.createElement("span");
      marker.className = "hotspot-marker";
      marker.setAttribute("aria-hidden", "true");
      marker.dataset.id = spot.id;
      position(marker, spot.rect);
      spots.appendChild(marker);
      load(destination(spot.to)).catch(function () {});
    });
    if (frame.cue && !visited[frame.scene === "footnote-page" ? "footnote" : frame.scene === "english-page" ? "english-popup" : "greek-popup"]) {
      var cue = document.createElement("span");
      cue.className = "demo-cue";
      cue.setAttribute("aria-hidden", "true");
      position(cue, frame.cue);
      spots.appendChild(cue);
    }
  }

  function setBusy(value) {
    busy = value;
    root.setAttribute("aria-busy", String(value));
    root.querySelectorAll(".hotspot, [data-advance], [data-restart], [data-sample]").forEach(function (button) {
      button.disabled = value;
    });
  }

  async function go(to, keyboard) {
    if (busy || !data.frames[to]) return;
    setBusy(true);
    var succeeded = false;
    try {
      await load(to);
      var previous = data.frames[current];
      var next = data.frames[to];
      var invert = previous.night !== next.night;
      render(to);
      if (invert && !reduce.matches && flash.animate) {
        await flash.animate([{ opacity: 0.3 }, { opacity: 0 }], { duration: 220 }).finished;
      }
      succeeded = true;
    } catch (_) {
      message("This screen couldn't load. Check your connection and try the action again.");
    } finally {
      setBusy(false);
    }
    if (succeeded && keyboard) guide.focus({ preventScroll: true });
  }

  guide.addEventListener("click", function (event) {
    if (suggested) go(destination(suggested.to), event.detail === 0);
  });

  root.addEventListener("keydown", function (event) {
    if (busy || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === "Escape" && root.classList.contains("is-expanded")) {
      event.preventDefault();
      expand(false);
      enlarge.focus({ preventScroll: true });
      return;
    }
    var spot = data.frames[current].hotspots.find(function (item) { return item.key === event.key; });
    if (spot) {
      event.preventDefault();
      go(destination(spot.to), true);
    }
  });

  function showActions(value) {
    root.classList.toggle("show-actions", value);
    reveal.setAttribute("aria-pressed", String(value));
    reveal.textContent = value ? "Hide actions" : "Show actions";
  }

  reveal.addEventListener("click", function () {
    showActions(reveal.getAttribute("aria-pressed") !== "true");
  });

  device.addEventListener("click", function (event) {
    if (busy || event.target.closest("button")) return;
    showActions(true);
    message("The marked areas work in this preview. You can also use the instruction above.");
  });

  function expand(value) {
    root.classList.toggle("is-expanded", value);
    enlarge.setAttribute("aria-pressed", String(value));
    enlarge.textContent = value ? "Reduce" : "Enlarge";
    guide.scrollIntoView({ block: "nearest" });
  }

  enlarge.addEventListener("click", function () { expand(!root.classList.contains("is-expanded")); });
  samples.addEventListener("click", function (event) {
    var button = event.target.closest("[data-sample]");
    if (button) go(button.dataset.sample + (data.frames[current].night ? "-night" : ""), event.detail === 0);
  });
  restart.addEventListener("click", function (event) {
    if (busy) return;
    var previousVisited = visited;
    var previousRead = lastRead;
    visited = {};
    lastRead = "greek-page";
    go(data.start, event.detail === 0).then(function () {
      if (current !== data.start) {
        visited = previousVisited;
        lastRead = previousRead;
      }
    });
  });

  guide.hidden = false;
  root.querySelector("[data-demo-controls]").hidden = false;
  root.demo = {
    get frame() { return current; },
    get busy() { return busy; }
  };
  render(data.start);
})();
