// Applies the saved theme before first paint so the page never flashes the wrong colors.
// Kept as a separate file because the Content-Security-Policy forbids inline scripts.
(function () {
  try {
    var t = localStorage.getItem("gantry.theme") || "system";
    var dark = t === "dark" || (t === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  } catch (e) {
    document.documentElement.dataset.theme = "light";
  }
})();
