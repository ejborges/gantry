// Applies the reader's choice of light or dark before the page is drawn, so it never flashes
// the other one. The choice is shared with the application when the manual is served by it.
// A file of its own because the content security policy forbids inline scripts.
(function () {
  try {
    var t = localStorage.getItem("gantry.theme");
    if (t === "light" || t === "dark") document.documentElement.dataset.theme = t;
  } catch (e) {
    /* no storage: the manual follows the device */
  }
})();
