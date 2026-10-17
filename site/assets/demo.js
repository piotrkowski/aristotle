// The Kindle demo: frames and hotspots from demo/frames.js.
//
// E-ink is shown honestly: a full update ("turn", "flash") darkens the screen
// for a moment and the new frame appears in a few discrete steps, about 0.4 s
// in all, like the panel's mode 9 and mode 8 updates. Menus ("overlay") appear
// without a flash. With reduced motion every change is an instant swap. Like
// the panel, the demo runs one update at a time: taps during an update are
// ignored.
(function () {
  "use strict";

  var data = window.ARISTOTLE_DEMO;
  var root = document.querySelector("[data-demo]");
  if (!data || !root) return;

  var W = data.screen.width;
  var H = data.screen.height;
  var img = root.querySelector(".device img");
  var spots = root.querySelector(".hotspots");
  var flash = root.querySelector(".flash");
  var hint = root.querySelector(".demo-hint");
  var stepLabel = root.querySelector(".demo-step");
  var restart = root.querySelector("[data-restart]");
  var note = root.querySelector(".demo-note");
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");

  // Flash timing, in ms: dark at once, the new frame at SWAP, then the dark
  // layer lifts in three steps, like gray levels settling.
  var TOTAL = 400;
  var SWAP = 140;
  var PEAK = { turn: 0.5, flash: 0.88 };

  var current = null;
  var busy = false;
  var step = 0;
  var visible = true;
  var cache = {};

  function base(path) {
    // Paths in the manifest are relative to the site root.
    return (root.getAttribute("data-root") || "") + path;
  }

  function load(id) {
    var f = data.frames[id];
    if (!cache[id]) {
      var im = new Image();
      im.src = base(f.image);
      cache[id] = im.decode ? im.decode().catch(function () {}) : Promise.resolve();
    }
    return cache[id];
  }

  function preloadNeighbours(id) {
    var f = data.frames[id];
    f.hotspots.forEach(function (h) { load(h.to); });
    var s = data.tour[step];
    if (s && s.action) load(s.action.to);
  }

  function pct(v, of) {
    return (v / of) * 100 + "%";
  }

  function render(id) {
    var f = data.frames[id];
    current = id;
    img.src = base(f.image);
    img.alt = f.alt;
    spots.textContent = "";
    var tour = data.tour[step];
    f.hotspots.forEach(function (h) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "hotspot";
      b.textContent = h.label;
      b.setAttribute("aria-label", h.label);
      b.dataset.id = h.id;
      var r = h.rect;
      b.style.left = pct(r[0], W);
      b.style.top = pct(r[1], H);
      b.style.width = pct(r[2], W);
      b.style.height = pct(r[3], H);
      if (tour && tour.frame === id && tour.target === h.id) {
        b.classList.add("is-suggested");
      }
      b.addEventListener("click", function () { go(h.to, h.kind, true); });
      spots.appendChild(b);
    });
    showHint();
    preloadNeighbours(id);
  }

  function showHint() {
    var s = data.tour[step];
    hint.textContent = "";
    if (!s || s.frame !== current) {
      hint.textContent = "Tap anywhere on the screen, or use Start again.";
      stepLabel.textContent = "";
      return;
    }
    var p = document.createElement("span");
    p.innerHTML = s.hint; // our own manifest; may hold <kbd>
    hint.appendChild(p);
    if (s.action) {
      hint.appendChild(document.createElement("br"));
      var b = document.createElement("button");
      b.type = "button";
      b.textContent = s.action.label;
      b.addEventListener("click", function () {
        go(s.action.to, s.action.kind, true);
      });
      hint.appendChild(b);
    }
    stepLabel.textContent = "Step " + (step + 1) + " of " + data.tour.length;
  }

  function advanceTour(to) {
    var next = data.tour[step + 1];
    if (next && next.frame === to) {
      step += 1;
    } else if (to === data.start && step === data.tour.length - 1) {
      step = 0;
    }
  }

  function focusAfter(hadFocus) {
    if (!hadFocus) return;
    var target = spots.querySelector(".is-suggested") ||
      hint.querySelector("button") || spots.querySelector(".hotspot");
    if (target) target.focus({ preventScroll: true });
  }

  function go(to, kind, fromUser) {
    if (busy || !to || !data.frames[to]) return;
    var hadFocus = root.contains(document.activeElement) &&
      document.activeElement !== document.body;
    advanceTour(to);
    var instant = reduce.matches || kind === "none" || kind === "overlay" ||
      !flash.animate;
    if (instant) {
      busy = true;
      load(to).then(function () {
        render(to);
        busy = false;
        focusAfter(hadFocus);
      });
      return;
    }
    busy = true;
    load(to).then(function () {
      var peak = PEAK[kind] || PEAK.turn;
      var anim = flash.animate(
        [
          { opacity: peak, offset: 0, easing: "steps(1, end)" },
          { opacity: peak, offset: SWAP / TOTAL, easing: "steps(1, end)" },
          { opacity: peak * 0.55, offset: 0.6, easing: "steps(1, end)" },
          { opacity: peak * 0.2, offset: 0.8, easing: "steps(1, end)" },
          { opacity: 0, offset: 1 }
        ],
        { duration: TOTAL, fill: "none" }
      );
      window.setTimeout(function () { render(to); }, SWAP);
      anim.finished.then(function () {
        busy = false;
        focusAfter(hadFocus);
      }, function () { busy = false; });
    });
  }

  function restartTour() {
    if (busy) return;
    step = 0;
    go(data.start, "flash", true);
  }

  // Arrow keys turn pages, Escape closes, while the demo is on screen.
  document.addEventListener("keydown", function (e) {
    if (!visible || busy || e.altKey || e.ctrlKey || e.metaKey) return;
    var t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    var f = data.frames[current];
    for (var i = 0; i < f.hotspots.length; i++) {
      var h = f.hotspots[i];
      if (h.key && h.key === e.key) {
        e.preventDefault();
        go(h.to, h.kind, true);
        return;
      }
    }
  });

  if ("IntersectionObserver" in window) {
    new IntersectionObserver(function (entries) {
      visible = entries[entries.length - 1].isIntersecting;
    }, { threshold: 0.25 }).observe(root);
  }

  if (restart) restart.addEventListener("click", restartTour);
  if (note && data.note) note.textContent = data.note;

  // Expose a tiny hook for scripted checks.
  root.demo = {
    get frame() { return current; },
    get step() { return step; },
    get busy() { return busy; }
  };

  render(data.start);
})();
