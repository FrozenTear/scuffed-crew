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
// itself, or an API call) cancels that fetch. Nothing failed: the page is
// going away. Chromium and most of the time Firefox reject it with an
// AbortError. Firefox 159 can instead reject it with
// "TypeError: NetworkError when attempting to fetch resource.", which is
// also what a real network failure says. So that one counts as a cancel
// only once a navigation away has started: `beforeunload` ran and nothing
// asked to stay (the unsaved-changes prompt prevents it). The page marks
// that on the root as `data-sc-leaving`, which the app's own error logging
// reads too (`util::is_aborted_fetch`). A key or pointer press clears it,
// since the page is then plainly still here (a download, a 204). Every
// other rejection still reaches the console.
(function () {
  var root = document.documentElement;
  var FETCH_FAILED = /NetworkError|Failed to fetch|Load failed/;
  function stay() {
    root.removeAttribute("data-sc-leaving");
  }
  window.addEventListener("beforeunload", function (event) {
    root.setAttribute("data-sc-leaving", "");
    // Runs after every other beforeunload listener: a prompt to stay
    // means the page is not leaving yet.
    setTimeout(function () {
      if (event.defaultPrevented) {
        stay();
      }
    }, 0);
  });
  window.addEventListener("pageshow", stay);
  window.addEventListener("keydown", stay, true);
  window.addEventListener("pointerdown", stay, true);
  window.addEventListener("unhandledrejection", function (event) {
    var reason = event.reason;
    if (!reason) {
      return;
    }
    var cancelled =
      reason.name === "AbortError" ||
      (reason.name === "TypeError" &&
        root.hasAttribute("data-sc-leaving") &&
        FETCH_FAILED.test(String(reason.message)));
    if (cancelled) {
      event.preventDefault();
    }
  });
})();
