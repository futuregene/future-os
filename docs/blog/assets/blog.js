// Theme toggle. The blog ships dark by default and follows the OS preference
// unless a choice was stored here — no framework, no network.
(function () {
  var root = document.documentElement;
  var stored = null;
  try {
    stored = window.localStorage.getItem("future-blog-theme");
  } catch (error) {
    stored = null;
  }
  if (stored === "light" || stored === "dark") {
    root.setAttribute("data-theme", stored);
  }

  document.querySelectorAll("[data-theme-toggle]").forEach(function (button) {
    button.addEventListener("click", function () {
      var current =
        root.getAttribute("data-theme") ||
        (window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark");
      var next = current === "light" ? "dark" : "light";
      root.setAttribute("data-theme", next);
      try {
        window.localStorage.setItem("future-blog-theme", next);
      } catch (error) {
        /* private mode: the toggle still works for this page view */
      }
    });
  });
})();
