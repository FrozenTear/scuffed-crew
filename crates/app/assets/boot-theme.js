// Paint the boot splash in the theme the member saved (sc-theme), before
// the app loads. It reads the value the way ThemeProvider does
// (crates/app/src/theme/provider.rs): "light" is light, any other saved
// value is dark, and no saved value leaves the OS preference to index.html.
// An external file, so the site CSP needs no new inline-script hash.
(function () {
  try {
    var saved = window.localStorage.getItem("sc-theme");
    if (saved !== null) {
      document.documentElement.setAttribute(
        "data-theme",
        saved === "light" ? "light" : "dark"
      );
    }
  } catch (e) {
    // Storage blocked: the OS preference decides, as before.
  }
})();
