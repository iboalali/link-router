(function () {
  const STORAGE_KEY = "linkRouterState";
  let cachedState = null;

  chrome.storage.local.get(STORAGE_KEY, (data) => {
    cachedState = data[STORAGE_KEY] || null;
  });

  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && changes[STORAGE_KEY]) {
      cachedState = changes[STORAGE_KEY].newValue || null;
    }
  });

  document.addEventListener(
    "click",
    (event) => {
      // Only intercept plain left-clicks without Alt (Alt bypasses routing once).
      if (event.defaultPrevented || event.button !== 0 || event.altKey) {
        return;
      }
      if (!cachedState || cachedState.paused) {
        return;
      }

      const anchor = event.target && event.target.closest ? event.target.closest("a[href]") : null;
      if (!anchor || !anchor.href) {
        return;
      }

      const url = anchor.href;
      if (!window.LinkRouterMatcher.shouldInterceptUrl(url, cachedState)) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();

      chrome.runtime.sendMessage({ type: "route_click", url }, (resp) => {
        if (chrome.runtime.lastError || !resp || !resp.ok || resp.action === "allow") {
          window.open(url, anchor.target || "_self", "noopener");
        }
      });
    },
    true
  );
})();
