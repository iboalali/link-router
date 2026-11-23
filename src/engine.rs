use crate::config::{Config, HandlerManifest, Rule, RuleAction};
use serde::Serialize;
use url::Url;

const MAX_UNWRAP_HOPS: usize = 3;
const BUILTIN_BYPASS_KEY: &str = "link_router";
const BUILTIN_BYPASS_VAL: &str = "browser";

const TRACKING_PARAMS: &[&str] = &[
    "fbclid",
    "gclid",
    "dclid",
    "mc_cid",
    "mc_eid",
    "igshid",
    "si",
    "yclid",
    "msclkid",
];

#[derive(Debug, Clone, Default)]
pub struct ResolveOptions<'a> {
    pub force_ask: bool,
    pub source_target: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    Open {
        target: String,
    },
    AllowInSource {
        target: String,
    },
    Ask {
        candidates: Vec<String>,
        suggested_host: String,
        suggested_path_prefix: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolutionResult {
    pub original_url: String,
    pub effective_url: String,
    pub unwrapped: bool,
    pub decision: Decision,
    pub trace: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionRuleSummary {
    pub id: String,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_glob: Option<String>,
    pub action: RuleAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

pub fn resolve(
    raw_url: &str,
    config: &Config,
    manifests: &[HandlerManifest],
    opts: ResolveOptions<'_>,
) -> Result<ResolutionResult, String> {
    let mut url =
        Url::parse(raw_url.trim()).map_err(|e| format!("invalid URL '{}': {}", raw_url, e))?;

    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("unsupported URL scheme '{}'", scheme));
    }

    let mut trace = Vec::new();
    let mut unwrapped = false;

    if config.unwrap_redirects {
        for _ in 0..MAX_UNWRAP_HOPS {
            if let Some(next) = unwrap_redirect_url(&url) {
                trace.push(format!("Unwrapped redirect {} -> {}", url, next));
                url = next;
                unwrapped = true;
            } else {
                break;
            }
        }
    }

    if config.strip_tracking_params {
        let (cleaned, changed) = strip_tracking_params(&url);
        if changed {
            trace.push(format!("Stripped tracking parameters -> {}", cleaned));
            url = cleaned;
        }
    }

    if !opts.force_ask {
        if let Some(marker) = matches_bypass_param(&url, &config.bypass_query_params) {
            trace.push(format!(
                "Bypass query parameter '{}' matched; opening in browser",
                marker
            ));
            let decision = match opts.source_target {
                Some(src) => Decision::AllowInSource {
                    target: src.to_string(),
                },
                None => Decision::Open {
                    target: config.fallback_target.clone(),
                },
            };
            return Ok(ResolutionResult {
                original_url: raw_url.to_string(),
                effective_url: url.to_string(),
                unwrapped,
                decision,
                trace,
            });
        }
    }

    let host = url
        .host_str()
        .ok_or_else(|| format!("URL has no host: {}", url))?
        .to_ascii_lowercase();
    let path = url.path();
    let suggested_prefix = suggested_path_prefix(&url);

    let mut matching_rules: Vec<((u8, usize, usize), &Rule)> = config
        .rules
        .iter()
        .enumerate()
        .filter(|(_, r)| r.enabled)
        .filter(|(_, r)| host_matches(&r.host, &host))
        .filter(|(_, r)| path_matches(r.path_prefix.as_deref(), r.path_glob.as_deref(), path))
        .map(|(idx, r)| {
            let host_rank = host_specificity(&r.host);
            let path_rank = path_specificity(r.path_prefix.as_deref(), r.path_glob.as_deref());
            ((host_rank, path_rank, usize::MAX - idx), r)
        })
        .collect();

    matching_rules.sort_by(|a, b| b.0.cmp(&a.0));

    if let Some((_, rule)) = matching_rules.first() {
        trace.push(format!(
            "Matched user rule '{}' (host='{}', action={:?})",
            rule.name, rule.host, rule.action
        ));

        let decision = match rule.action {
            RuleAction::Always => {
                let target = rule
                    .target
                    .clone()
                    .unwrap_or_else(|| config.fallback_target.clone());
                if opts.force_ask {
                    trace.push("Forced chooser via --ask".to_string());
                    Decision::Ask {
                        candidates: dedup_candidates(vec![target, config.fallback_target.clone()]),
                        suggested_host: host,
                        suggested_path_prefix: suggested_prefix,
                    }
                } else {
                    finalize_open(target, opts.source_target, &mut trace)
                }
            }
            RuleAction::Ask => {
                let mut candidates = rule.candidates.clone();
                if let Some(t) = &rule.target {
                    candidates.insert(0, t.clone());
                }
                if !candidates.contains(&config.fallback_target) {
                    candidates.push(config.fallback_target.clone());
                }
                Decision::Ask {
                    candidates: dedup_candidates(candidates),
                    suggested_host: host,
                    suggested_path_prefix: suggested_prefix,
                }
            }
            RuleAction::Browser => {
                if opts.force_ask {
                    trace.push("Forced chooser via --ask".to_string());
                    Decision::Ask {
                        candidates: vec![config.fallback_target.clone()],
                        suggested_host: host,
                        suggested_path_prefix: suggested_prefix,
                    }
                } else {
                    trace.push("Rule delegates to fallback browser".to_string());
                    finalize_open(config.fallback_target.clone(), opts.source_target, &mut trace)
                }
            }
        };

        return Ok(ResolutionResult {
            original_url: raw_url.to_string(),
            effective_url: url.to_string(),
            unwrapped,
            decision,
            trace,
        });
    }

    let enabled_matches: Vec<&HandlerManifest> = manifests
        .iter()
        .filter(|m| config.is_handler_enabled(&m.id))
        .filter(|m| {
            m.domains.iter().any(|d| {
                host_matches(&d.host, &host)
                    && path_matches(d.path_prefix.as_deref(), d.path_glob.as_deref(), path)
            })
        })
        .collect();

    let decision = if opts.force_ask {
        let mut candidates: Vec<String> = enabled_matches.iter().map(|m| m.target.clone()).collect();
        candidates.push(config.fallback_target.clone());
        trace.push("Forced chooser via --ask".to_string());
        Decision::Ask {
            candidates: dedup_candidates(candidates),
            suggested_host: host,
            suggested_path_prefix: suggested_prefix,
        }
    } else if enabled_matches.is_empty() {
        trace.push(format!(
            "No matching rule or enabled handler; using fallback '{}'",
            config.fallback_target
        ));
        finalize_open(config.fallback_target.clone(), opts.source_target, &mut trace)
    } else if enabled_matches.len() == 1 {
        let m = enabled_matches[0];
        trace.push(format!(
            "Matched enabled handler '{}' -> '{}'",
            m.name, m.target
        ));
        finalize_open(m.target.clone(), opts.source_target, &mut trace)
    } else {
        trace.push(format!(
            "Matched {} enabled handlers; prompting chooser",
            enabled_matches.len()
        ));
        let mut candidates: Vec<String> = enabled_matches.iter().map(|m| m.target.clone()).collect();
        candidates.push(config.fallback_target.clone());
        Decision::Ask {
            candidates: dedup_candidates(candidates),
            suggested_host: host,
            suggested_path_prefix: suggested_prefix,
        }
    };

    Ok(ResolutionResult {
        original_url: raw_url.to_string(),
        effective_url: url.to_string(),
        unwrapped,
        decision,
        trace,
    })
}

fn finalize_open(
    target: String,
    source_target: Option<&str>,
    trace: &mut Vec<String>,
) -> Decision {
    if source_target == Some(target.as_str()) {
        trace.push(format!(
            "Source '{}' already matches target '{}'; allowing in place",
            target, target
        ));
        Decision::AllowInSource { target }
    } else {
        Decision::Open { target }
    }
}

fn dedup_candidates(items: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

pub fn export_extension_rules(
    config: &Config,
    manifests: &[HandlerManifest],
) -> Vec<ExtensionRuleSummary> {
    let mut out = Vec::new();
    for rule in &config.rules {
        if !rule.enabled {
            continue;
        }
        out.push(ExtensionRuleSummary {
            id: rule.id.clone(),
            host: rule.host.clone(),
            path_prefix: rule.path_prefix.clone(),
            path_glob: rule.path_glob.clone(),
            action: rule.action.clone(),
            target: match rule.action {
                RuleAction::Always => rule.target.clone(),
                RuleAction::Browser => Some(config.fallback_target.clone()),
                RuleAction::Ask => None,
            },
        });
    }
    for manifest in manifests {
        if !config.is_handler_enabled(&manifest.id) {
            continue;
        }
        for domain in &manifest.domains {
            out.push(ExtensionRuleSummary {
                id: format!("handler:{}", manifest.id),
                host: domain.host.clone(),
                path_prefix: domain.path_prefix.clone(),
                path_glob: domain.path_glob.clone(),
                action: RuleAction::Always,
                target: Some(manifest.target.clone()),
            });
        }
    }
    out
}

pub fn unwrap_redirect_url(url: &Url) -> Option<Url> {
    let host = url.host_str()?.to_ascii_lowercase();
    let path = url.path();

    let param_name = if (host == "www.google.com" || host == "google.com") && path == "/url" {
        Some(&["q", "url"][..])
    } else if host.ends_with(".safelinks.protection.outlook.com")
        || host == "safelinks.protection.outlook.com"
    {
        Some(&["url"][..])
    } else if host == "slack-redir.net" && path.starts_with("/link") {
        Some(&["url"][..])
    } else if (host == "l.facebook.com" || host == "l.messenger.com") && path == "/l.php" {
        Some(&["u"][..])
    } else if host == "www.linkedin.com" && path.starts_with("/redir/redirect") {
        Some(&["url"][..])
    } else {
        None
    }?;

    for (k, v) in url.query_pairs() {
        if param_name.iter().any(|p| *p == k) {
            if let Ok(parsed) = Url::parse(&v) {
                if parsed.scheme() == "http" || parsed.scheme() == "https" {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

pub fn strip_tracking_params(url: &Url) -> (Url, bool) {
    let mut cleaned = url.clone();
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut removed = false;

    for (k, v) in url.query_pairs() {
        let lower = k.to_ascii_lowercase();
        let is_tracking =
            lower.starts_with("utm_") || TRACKING_PARAMS.iter().any(|p| *p == lower);
        if is_tracking {
            removed = true;
        } else {
            kept.push((k.into_owned(), v.into_owned()));
        }
    }

    if !removed {
        return (cleaned, false);
    }

    if kept.is_empty() {
        cleaned.set_query(None);
    } else {
        let mut pairs = cleaned.query_pairs_mut();
        pairs.clear();
        for (k, v) in kept {
            pairs.append_pair(&k, &v);
        }
    }
    (cleaned, true)
}

pub fn host_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().to_ascii_lowercase();
    let h = host.trim().to_ascii_lowercase();
    if p == "*" {
        return true;
    }
    if let Some(suffix) = p.strip_prefix("*.") {
        return h.len() > suffix.len()
            && h.ends_with(suffix)
            && h.as_bytes()[h.len() - suffix.len() - 1] == b'.';
    }
    p == h
}

fn host_specificity(pattern: &str) -> u8 {
    let p = pattern.trim();
    if p == "*" {
        0
    } else if p.starts_with("*.") {
        1
    } else {
        2
    }
}

pub fn path_matches(path_prefix: Option<&str>, path_glob: Option<&str>, path: &str) -> bool {
    if let Some(prefix) = path_prefix {
        let p = prefix.trim();
        if !p.is_empty() && p != "/" && !path.starts_with(p) {
            return false;
        }
    }
    if let Some(glob) = path_glob {
        let g = glob.trim();
        if !g.is_empty() && !glob_match(g, path) {
            return false;
        }
    }
    true
}

fn path_specificity(path_prefix: Option<&str>, path_glob: Option<&str>) -> usize {
    let mut score = 0;
    if let Some(p) = path_prefix {
        let trimmed = p.trim();
        if !trimmed.is_empty() && trimmed != "/" {
            score += trimmed.len() + 1;
        }
    }
    if let Some(g) = path_glob {
        let trimmed = g.trim();
        if !trimmed.is_empty() {
            score += trimmed.len() + 1;
        }
    }
    score
}

pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star_idx: Option<usize> = None;
    let mut match_ti = 0;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_ti = ti;
            pi += 1;
        } else if let Some(si) = star_idx {
            pi = si + 1;
            match_ti += 1;
            ti = match_ti;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn suggested_path_prefix(url: &Url) -> Option<String> {
    let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();
    segments.first().map(|first| format!("/{}/", first))
}

pub fn append_builtin_bypass_param(raw_url: &str) -> String {
    let Ok(mut url) = Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    if matches_bypass_param(&url, &[]).is_some() {
        return url.to_string();
    }
    url.query_pairs_mut()
        .append_pair(BUILTIN_BYPASS_KEY, BUILTIN_BYPASS_VAL);
    url.to_string()
}

pub fn matches_bypass_param(url: &Url, extra_params: &[String]) -> Option<String> {
    for (k, v) in url.query_pairs() {
        if k == BUILTIN_BYPASS_KEY && v == BUILTIN_BYPASS_VAL {
            return Some(format!("{}={}", BUILTIN_BYPASS_KEY, BUILTIN_BYPASS_VAL));
        }
        for raw in extra_params {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((ek, ev)) = trimmed.split_once('=') {
                if k == ek && v == ev {
                    return Some(trimmed.to_string());
                }
            } else if k == trimmed {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DomainPattern;

    fn jira_config() -> Config {
        let mut cfg = Config::default();
        cfg.fallback_target = "chrome:Default".to_string();
        cfg.rules.push(Rule {
            id: "work-jira".to_string(),
            name: "Work Jira".to_string(),
            enabled: true,
            host: "*.atlassian.net".to_string(),
            path_prefix: None,
            path_glob: None,
            action: RuleAction::Always,
            target: Some("chrome:Profile 1".to_string()),
            candidates: Vec::new(),
        });
        cfg
    }

    #[test]
    fn opens_fallback_browser_by_default() {
        let cfg = Config::default();
        let manifests = vec![HandlerManifest {
            id: "jira-app".to_string(),
            name: "Jira".to_string(),
            target: "chrome:Profile 1".to_string(),
            domains: vec![DomainPattern {
                host: "*.atlassian.net".to_string(),
                path_prefix: None,
                path_glob: None,
            }],
        }];

        let res = resolve(
            "https://acme.atlassian.net/browse/TM-123",
            &cfg,
            &manifests,
            ResolveOptions::default(),
        )
        .unwrap();
        assert_eq!(
            res.decision,
            Decision::Open {
                target: "desktop:google-chrome.desktop".to_string()
            }
        );
    }

    #[test]
    fn routes_jira_url_to_work_profile() {
        let cfg = jira_config();
        let res = resolve(
            "https://acme.atlassian.net/browse/TM-123",
            &cfg,
            &[],
            ResolveOptions {
                force_ask: false,
                source_target: Some("chrome:Default"),
            },
        )
        .unwrap();
        assert_eq!(
            res.decision,
            Decision::Open {
                target: "chrome:Profile 1".to_string()
            }
        );
    }

    #[test]
    fn allows_in_place_when_source_matches_target() {
        let cfg = jira_config();
        let res = resolve(
            "https://acme.atlassian.net/browse/TM-123",
            &cfg,
            &[],
            ResolveOptions {
                force_ask: false,
                source_target: Some("chrome:Profile 1"),
            },
        )
        .unwrap();
        assert_eq!(
            res.decision,
            Decision::AllowInSource {
                target: "chrome:Profile 1".to_string()
            }
        );
    }

    #[test]
    fn specificity_prefers_exact_host_and_path_over_wildcard() {
        let mut cfg = jira_config();
        cfg.rules.push(Rule {
            id: "wiki-ask".to_string(),
            name: "Ask on wiki".to_string(),
            enabled: true,
            host: "acme.atlassian.net".to_string(),
            path_prefix: Some("/wiki/".to_string()),
            path_glob: None,
            action: RuleAction::Ask,
            target: Some("chrome:Profile 1".to_string()),
            candidates: vec!["firefox:default".to_string()],
        });

        let res = resolve(
            "https://acme.atlassian.net/wiki/spaces/ENG",
            &cfg,
            &[],
            ResolveOptions::default(),
        )
        .unwrap();
        assert!(matches!(res.decision, Decision::Ask { .. }));
    }

    #[test]
    fn unwraps_google_and_slack_redirects_before_matching() {
        let cfg = jira_config();
        let wrapped = "https://www.google.com/url?q=https%3A%2F%2Facme.atlassian.net%2Fbrowse%2FTM-42&sa=D";
        let res = resolve(wrapped, &cfg, &[], ResolveOptions::default()).unwrap();
        assert!(res.unwrapped);
        assert_eq!(res.effective_url, "https://acme.atlassian.net/browse/TM-42");
        assert_eq!(
            res.decision,
            Decision::Open {
                target: "chrome:Profile 1".to_string()
            }
        );
    }

    #[test]
    fn strips_tracking_parameters_when_enabled() {
        let mut cfg = jira_config();
        cfg.strip_tracking_params = true;
        let res = resolve(
            "https://acme.atlassian.net/browse/TM-9?utm_source=slack&focusedCommentId=5",
            &cfg,
            &[],
            ResolveOptions::default(),
        )
        .unwrap();
        assert_eq!(
            res.effective_url,
            "https://acme.atlassian.net/browse/TM-9?focusedCommentId=5"
        );
    }

    #[test]
    fn prompts_when_multiple_enabled_handlers_match() {
        let mut cfg = Config::default();
        cfg.set_handler_enabled("h1", true);
        cfg.set_handler_enabled("h2", true);
        let manifests = vec![
            HandlerManifest {
                id: "h1".to_string(),
                name: "Work Chrome".to_string(),
                target: "chrome:Profile 1".to_string(),
                domains: vec![DomainPattern {
                    host: "*.atlassian.net".to_string(),
                    path_prefix: None,
                    path_glob: None,
                }],
            },
            HandlerManifest {
                id: "h2".to_string(),
                name: "TempoUI".to_string(),
                target: "desktop:com.example.jira.desktop".to_string(),
                domains: vec![DomainPattern {
                    host: "*.atlassian.net".to_string(),
                    path_prefix: None,
                    path_glob: None,
                }],
            },
        ];

        let res = resolve(
            "https://acme.atlassian.net/browse/TM-1",
            &cfg,
            &manifests,
            ResolveOptions::default(),
        )
        .unwrap();
        match res.decision {
            Decision::Ask { candidates, .. } => {
                assert!(candidates.contains(&"chrome:Profile 1".to_string()));
                assert!(candidates.contains(&"desktop:com.example.jira.desktop".to_string()));
            }
            other => panic!("expected Ask, got {:?}", other),
        }
    }

    #[test]
    fn bypasses_routing_for_builtin_and_configured_query_params() {
        let mut cfg = jira_config();
        cfg.bypass_query_params = vec!["betterjira=browser".to_string()];

        let desktop_builtin = resolve(
            "https://acme.atlassian.net/browse/TM-1?link_router=browser",
            &cfg,
            &[],
            ResolveOptions::default(),
        )
        .unwrap();
        assert_eq!(
            desktop_builtin.decision,
            Decision::Open {
                target: "chrome:Default".to_string()
            }
        );

        let browser_custom = resolve(
            "https://acme.atlassian.net/browse/TM-1?focusedCommentId=5&betterjira=browser#comment-5",
            &cfg,
            &[],
            ResolveOptions {
                force_ask: false,
                source_target: Some("chrome:Default"),
            },
        )
        .unwrap();
        assert_eq!(
            browser_custom.decision,
            Decision::AllowInSource {
                target: "chrome:Default".to_string()
            }
        );

        let partial_value = resolve(
            "https://acme.atlassian.net/browse/TM-1?betterjira=browsers",
            &cfg,
            &[],
            ResolveOptions::default(),
        )
        .unwrap();
        assert_eq!(
            partial_value.decision,
            Decision::Open {
                target: "chrome:Profile 1".to_string()
            }
        );

        let in_fragment = resolve(
            "https://acme.atlassian.net/browse/TM-1#?betterjira=browser",
            &cfg,
            &[],
            ResolveOptions::default(),
        )
        .unwrap();
        assert_eq!(
            in_fragment.decision,
            Decision::Open {
                target: "chrome:Profile 1".to_string()
            }
        );
    }

    #[test]
    fn appends_builtin_bypass_param_cleanly() {
        assert_eq!(
            append_builtin_bypass_param("https://acme.atlassian.net/browse/TM-1"),
            "https://acme.atlassian.net/browse/TM-1?link_router=browser"
        );
        assert_eq!(
            append_builtin_bypass_param("https://acme.atlassian.net/browse/TM-1?foo=1#c"),
            "https://acme.atlassian.net/browse/TM-1?foo=1&link_router=browser#c"
        );
        assert_eq!(
            append_builtin_bypass_param(
                "https://acme.atlassian.net/browse/TM-1?link_router=browser"
            ),
            "https://acme.atlassian.net/browse/TM-1?link_router=browser"
        );
    }
}
