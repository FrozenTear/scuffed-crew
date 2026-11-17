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

// QA 696 nit 8: leaving a page while a fetch is in flight (the app bundle
// itself, or an API call) makes Firefox reject that fetch with an
// AbortError. Nothing failed: the page is going away. Mark only that
// rejection handled, so it is not reported as an uncaught error. Every
// other rejection still reaches the console.
window.addEventListener("unhandledrejection", function (event) {
  var reason = event.reason;
  if (reason && reason.name === "AbortError") {
    event.preventDefault();
  }
});
