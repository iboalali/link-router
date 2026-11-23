// Shared URL rule matcher for Link Router's content script and service worker.
(function (root) {
  function hostMatches(pattern, host) {
    const p = (pattern || "").trim().toLowerCase();
    const h = (host || "").trim().toLowerCase();
    if (!p || !h) return false;
    if (p === "*") return true;
    if (p.startsWith("*.")) {
      const suffix = p.slice(2);
      return h.length > suffix.length && h.endsWith("." + suffix);
    }
    return p === h;
  }

  function globToRegex(glob) {
    const escaped = glob.replace(/[.+^${}()|[\]\\]/g, "\\$&");
    const regexStr = "^" + escaped.replace(/\*/g, ".*").replace(/\?/g, ".") + "$";
    return new RegExp(regexStr);
  }

  function pathMatches(rule, pathname) {
    if (rule.path_prefix && rule.path_prefix !== "/") {
      if (!pathname.startsWith(rule.path_prefix)) return false;
    }
    if (rule.path_glob) {
      if (!globToRegex(rule.path_glob).test(pathname)) return false;
    }
    return true;
  }

  function hostSpecificity(pattern) {
    const p = (pattern || "").trim();
    if (p === "*") return 0;
    if (p.startsWith("*.")) return 1;
    return 2;
  }

  function pathSpecificity(rule) {
    let score = 0;
    if (rule.path_prefix && rule.path_prefix !== "/") {
      score += rule.path_prefix.length + 1;
    }
    if (rule.path_glob) {
      score += rule.path_glob.length + 1;
    }
    return score;
  }

  function unwrapKnownRedirect(urlObj) {
    const host = urlObj.hostname.toLowerCase();
    const path = urlObj.pathname;
    let params = null;
    if ((host === "www.google.com" || host === "google.com") && path === "/url") {
      params = ["q", "url"];
    } else if (
      host.endsWith(".safelinks.protection.outlook.com") ||
      host === "safelinks.protection.outlook.com"
    ) {
      params = ["url"];
    } else if (host === "slack-redir.net" && path.startsWith("/link")) {
      params = ["url"];
    } else if ((host === "l.facebook.com" || host === "l.messenger.com") && path === "/l.php") {
      params = ["u"];
    } else if (host === "www.linkedin.com" && path.startsWith("/redir/redirect")) {
      params = ["url"];
    }
    if (!params) return null;
    for (const key of params) {
      const raw = urlObj.searchParams.get(key);
      if (!raw) continue;
      try {
        const next = new URL(raw);
        if (next.protocol === "http:" || next.protocol === "https:") {
          return next;
        }
      } catch (_) {}
    }
    return null;
  }

  function hasBypassParam(urlObj, extraParams) {
    if (urlObj.searchParams.getAll("link_router").includes("browser")) {
      return true;
    }
    if (!Array.isArray(extraParams)) return false;
    for (const raw of extraParams) {
      const trimmed = (raw || "").trim();
      if (!trimmed) continue;
      const eq = trimmed.indexOf("=");
      if (eq >= 0) {
        const key = trimmed.slice(0, eq);
        const val = trimmed.slice(eq + 1);
        if (urlObj.searchParams.getAll(key).includes(val)) return true;
      } else if (urlObj.searchParams.has(trimmed)) {
        return true;
      }
    }
    return false;
  }

  function shouldInterceptUrl(rawUrl, state) {
    if (!state || state.paused || !Array.isArray(state.rules) || state.rules.length === 0) {
      return false;
    }

    let parsed;
    try {
      parsed = new URL(rawUrl);
    } catch (_) {
      return false;
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return false;
    }

    if (state.unwrap_redirects !== false) {
      for (let i = 0; i < 3; i++) {
        const next = unwrapKnownRedirect(parsed);
        if (!next) break;
        parsed = next;
      }
    }

    if (hasBypassParam(parsed, state.bypass_query_params)) {
      return false;
    }

    const host = parsed.hostname.toLowerCase();
    const pathname = parsed.pathname;

    let bestRule = null;
    let bestScore = null;
    for (let i = 0; i < state.rules.length; i++) {
      const rule = state.rules[i];
      if (!hostMatches(rule.host, host)) continue;
      if (!pathMatches(rule, pathname)) continue;
      const score = [hostSpecificity(rule.host), pathSpecificity(rule), -i];
      if (
        !bestScore ||
        score[0] > bestScore[0] ||
        (score[0] === bestScore[0] && score[1] > bestScore[1]) ||
        (score[0] === bestScore[0] && score[1] === bestScore[1] && score[2] > bestScore[2])
      ) {
        bestScore = score;
        bestRule = rule;
      }
    }

    if (!bestRule) return false;
    if (bestRule.action === "ask") return true;

    const currentTarget = state.source_profile ? "chrome:" + state.source_profile : null;
    if (bestRule.target && currentTarget && bestRule.target === currentTarget) {
      return false;
    }
    return true;
  }

  root.LinkRouterMatcher = {
    hostMatches,
    pathMatches,
    unwrapKnownRedirect,
    hasBypassParam,
    shouldInterceptUrl,
  };
})(typeof self !== "undefined" ? self : globalThis);
