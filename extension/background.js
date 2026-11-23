importScripts("matcher.js");

const NATIVE_HOST = "com.iboalali.link_router";
const STORAGE_KEY = "linkRouterState";

const DEFAULT_STATE = {
  connected: false,
  lastError: null,
  paused: false,
  source_profile: null,
  fallback_target: null,
  unwrap_redirects: true,
  bypass_query_params: [],
  profiles: [],
  rules: [],
};

// Track tabs created recently so a redirected window.open tab can be closed cleanly.
const recentNewTabs = new Map();
// Tabs where a redirected navigation was still pending when tabs.update failed (e.g. file:// without file access).
const pendingGoBackTabs = new Map();

function isBlankOrNewTab(url) {
  return (
    !url ||
    url === "about:blank" ||
    url.startsWith("chrome://newtab") ||
    url.startsWith("chrome-search://")
  );
}

function sendNative(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendNativeMessage(NATIVE_HOST, message, (response) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
      } else {
        resolve(response);
      }
    });
  });
}

async function getState() {
  const stored = await chrome.storage.local.get(STORAGE_KEY);
  return Object.assign({}, DEFAULT_STATE, stored[STORAGE_KEY] || {});
}

async function saveState(patch) {
  const current = await getState();
  const next = Object.assign({}, current, patch);
  await chrome.storage.local.set({ [STORAGE_KEY]: next });
  return next;
}

function getProfileEmail() {
  return new Promise((resolve) => {
    if (!chrome.identity || !chrome.identity.getProfileUserInfo) {
      resolve(null);
      return;
    }
    chrome.identity.getProfileUserInfo({ accountStatus: "ANY" }, (info) => {
      if (chrome.runtime.lastError || !info || !info.email) {
        resolve(null);
      } else {
        resolve(info.email.toLowerCase());
      }
    });
  });
}

async function refreshStateFromHost() {
  try {
    const resp = await sendNative({ type: "get_state" });
    if (!resp || !resp.ok) {
      return await saveState({
        connected: false,
        lastError: (resp && resp.error) || "Native host returned an error",
      });
    }

    const current = await getState();
    let sourceProfile = current.source_profile;
    const profiles = Array.isArray(resp.profiles) ? resp.profiles : [];

    if (!sourceProfile) {
      const email = await getProfileEmail();
      if (email) {
        const match = profiles.find(
          (p) => p.email && p.email.toLowerCase() === email && p.profile_dir
        );
        if (match) {
          sourceProfile = match.profile_dir;
        }
      }
      if (!sourceProfile && profiles.length === 1 && profiles[0].profile_dir) {
        sourceProfile = profiles[0].profile_dir;
      }
    }

    return await saveState({
      connected: true,
      lastError: null,
      source_profile: sourceProfile,
      fallback_target: resp.fallback_target || null,
      unwrap_redirects: resp.unwrap_redirects !== false,
      bypass_query_params: Array.isArray(resp.bypass_query_params)
        ? resp.bypass_query_params
        : [],
      profiles,
      rules: Array.isArray(resp.rules) ? resp.rules : [],
    });
  } catch (err) {
    return await saveState({
      connected: false,
      lastError: err.message,
    });
  }
}

async function routeUrl(url, forceAsk) {
  const state = await getState();
  if (state.paused) {
    return { ok: true, action: "allow" };
  }
  try {
    const resp = await sendNative({
      type: "route",
      url,
      source_profile: state.source_profile || "",
      force_ask: Boolean(forceAsk),
    });
    if (resp && resp.ok && resp.action === "redirected") {
      // Refresh cached rules in case the user clicked "Always" in the chooser.
      refreshStateFromHost();
    }
    return resp || { ok: false, error: "Empty response" };
  } catch (err) {
    await saveState({ connected: false, lastError: err.message });
    return { ok: false, error: err.message };
  }
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.alarms.create("link-router-refresh", { periodInMinutes: 5 });
  refreshStateFromHost();
});

chrome.runtime.onStartup.addListener(() => {
  refreshStateFromHost();
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "link-router-refresh") {
    refreshStateFromHost();
  }
});

chrome.tabs.onCreated.addListener((tab) => {
  if (tab.id != null) {
    recentNewTabs.set(tab.id, Date.now());
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  recentNewTabs.delete(tabId);
  pendingGoBackTabs.delete(tabId);
});

chrome.webNavigation.onCommitted.addListener((details) => {
  if (details.frameId !== 0) return;
  const pending = pendingGoBackTabs.get(details.tabId);
  if (!pending) return;
  pendingGoBackTabs.delete(details.tabId);
  if (Date.now() <= pending.expiresAt && details.url !== pending.previousUrl) {
    chrome.tabs.goBack(details.tabId).catch(() => {});
  }
});

chrome.webNavigation.onErrorOccurred.addListener((details) => {
  if (details.frameId !== 0) return;
  pendingGoBackTabs.delete(details.tabId);
});

chrome.webNavigation.onBeforeNavigate.addListener(async (details) => {
  if (details.frameId !== 0) return;
  const state = await getState();
  if (!self.LinkRouterMatcher.shouldInterceptUrl(details.url, state)) {
    return;
  }

  let previousUrl = null;
  try {
    const tabBefore = await chrome.tabs.get(details.tabId);
    if (tabBefore && tabBefore.url && tabBefore.url !== details.url) {
      previousUrl = tabBefore.url;
    }
  } catch {
    // Tab may have closed already.
  }

  const resp = await routeUrl(details.url, false);
  if (resp && resp.ok && resp.action === "redirected") {
    const hasPreviousPage = Boolean(previousUrl) && !isBlankOrNewTab(previousUrl);
    const createdAt = recentNewTabs.get(details.tabId);
    if (!hasPreviousPage && createdAt && Date.now() - createdAt < 5000) {
      recentNewTabs.delete(details.tabId);
      chrome.tabs.remove(details.tabId).catch(() => {});
      return;
    }

    if (hasPreviousPage) {
      let tabAfter = null;
      try {
        tabAfter = await chrome.tabs.get(details.tabId);
      } catch {
        return;
      }

      if (tabAfter.url === previousUrl) {
        try {
          await chrome.tabs.update(details.tabId, { url: previousUrl });
          return;
        } catch {
          try {
            const tabNow = await chrome.tabs.get(details.tabId);
            if (tabNow.url !== previousUrl) {
              chrome.tabs.goBack(details.tabId).catch(() => {});
            } else {
              pendingGoBackTabs.set(details.tabId, {
                previousUrl,
                expiresAt: Date.now() + 15000,
              });
            }
          } catch {
            // Tab closed.
          }
          return;
        }
      }

      chrome.tabs.goBack(details.tabId).catch(() => {
        chrome.tabs.update(details.tabId, { url: previousUrl }).catch(() => {});
      });
      return;
    }

    if (previousUrl && isBlankOrNewTab(previousUrl)) {
      chrome.tabs.remove(details.tabId).catch(() => {});
    } else {
      chrome.tabs.goBack(details.tabId).catch(() => {
        chrome.tabs.remove(details.tabId).catch(() => {});
      });
    }
  }
});

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg || typeof msg !== "object") return;

  if (msg.type === "route_click") {
    routeUrl(msg.url, Boolean(msg.force_ask)).then(sendResponse);
    return true;
  }

  if (msg.type === "popup_get_state") {
    refreshStateFromHost().then(sendResponse);
    return true;
  }

  if (msg.type === "popup_set_profile") {
    saveState({ source_profile: msg.profile_dir || null }).then(sendResponse);
    return true;
  }

  if (msg.type === "popup_set_paused") {
    saveState({ paused: Boolean(msg.paused) }).then(sendResponse);
    return true;
  }

  if (msg.type === "popup_open_manager") {
    sendNative({ type: "open_manager" })
      .then(sendResponse)
      .catch((err) => sendResponse({ ok: false, error: err.message }));
    return true;
  }
});
