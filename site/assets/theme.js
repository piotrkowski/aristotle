(function () {
  "use strict";

  var root = document.documentElement;
  var key = "aristotle-theme";

  try {
    if (localStorage.getItem(key) === "dark") root.dataset.theme = "dark";
  } catch (_) {}

  document.addEventListener("DOMContentLoaded", function () {
    var button = document.querySelector("[data-theme-toggle]");
    if (!button) return;

    function update() {
      var dark = root.dataset.theme === "dark";
      button.setAttribute("aria-label", dark ? "Switch to light mode" : "Switch to dark mode");
      button.title = button.getAttribute("aria-label");
    }

    button.hidden = false;
    update();
    button.addEventListener("click", function () {
      root.dataset.theme = root.dataset.theme === "dark" ? "light" : "dark";
      try {
        localStorage.setItem(key, root.dataset.theme);
      } catch (_) {}
      update();
    });
  });
})();
