(function () {
  "use strict";

  var data = window.ARISTOTLE_DEMO;
  var root = document.querySelector("[data-demo]");
  if (!data || !root) return;

  var img = root.querySelector(".device img");
  var spots = root.querySelector(".hotspots");
  var flash = root.querySelector(".flash");
  var hint = root.querySelector(".demo-hint");
  var restart = root.querySelector("[data-restart]");
  var invite = root.querySelector(".demo-invite");
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  var current = data.start;
  var busy = false;
  var cache = {};

  function base(path) {
    return (root.getAttribute("data-root") || "") + path;
  }

  function load(id) {
    if (!cache[id]) {
      cache[id] = new Promise(function (resolve, reject) {
        var image = new Image();
        image.onload = resolve;
        image.onerror = reject;
        image.src = base(data.frames[id].image);
      }).catch(function (error) {
        delete cache[id];
        throw error;
      });
    }
    return cache[id];
  }

  function render(id) {
    var frame = data.frames[id];
    current = id;
    if (invite) invite.hidden = id !== data.start;
    img.src = base(frame.image);
    img.alt = frame.alt;
    hint.textContent = frame.hint;
    spots.textContent = "";
    frame.hotspots.forEach(function (spot) {
      var button = document.createElement("button");
      button.type = "button";
      button.className = "hotspot";
      button.setAttribute("aria-label", spot.label);
      button.dataset.id = spot.id;
      button.style.left = spot.rect[0] / data.screen.width * 100 + "%";
      button.style.top = spot.rect[1] / data.screen.height * 100 + "%";
      button.style.width = spot.rect[2] / data.screen.width * 100 + "%";
      button.style.height = spot.rect[3] / data.screen.height * 100 + "%";
      button.addEventListener("click", function (event) {
        go(spot.to, spot.kind, event.detail === 0);
      });
      spots.appendChild(button);
      load(spot.to).catch(function () {});
    });
  }

  function setBusy(value) {
    busy = value;
    restart.disabled = value;
    root.setAttribute("aria-busy", String(value));
  }

  function focusScreen(keyboard) {
    if (keyboard) spots.querySelector("button").focus({ preventScroll: true });
  }

  async function go(to, kind, keyboard) {
    if (busy || !data.frames[to]) return;
    setBusy(true);
    try {
      await load(to);
      if (reduce.matches || kind === "none" || kind === "overlay" || !flash.animate) {
        render(to);
      } else {
        var peak = kind === "flash" ? 0.65 : 0.3;
        var animation = flash.animate([
          { opacity: peak, offset: 0 },
          { opacity: peak, offset: 0.35, easing: "steps(1, end)" },
          { opacity: peak * 0.4, offset: 0.65, easing: "steps(1, end)" },
          { opacity: 0, offset: 1 }
        ], { duration: 400 });
        await new Promise(function (resolve) { window.setTimeout(resolve, 140); });
        render(to);
        await animation.finished;
      }
      focusScreen(keyboard);
    } catch (_) {
      hint.textContent = "Couldn't load this screen. Please try again.";
    } finally {
      setBusy(false);
    }
  }

  root.addEventListener("keydown", function (event) {
    if (busy || event.altKey || event.ctrlKey || event.metaKey) return;
    var spot = data.frames[current].hotspots.find(function (item) {
      return item.key === event.key;
    });
    if (spot) {
      event.preventDefault();
      go(spot.to, spot.kind, true);
    }
  });

  restart.hidden = false;
  restart.addEventListener("click", function (event) {
    go(data.start, "flash", event.detail === 0);
  });

  root.demo = {
    get frame() { return current; },
    get busy() { return busy; }
  };

  render(data.start);
})();
