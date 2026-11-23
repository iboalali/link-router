# Link Router Design

Link Router is an Android-style URL and link router for Linux desktops (GNOME/Wayland) and multi-profile Chrome setups.

## Goals

- Intercept `http://` and `https://` links system-wide via `x-scheme-handler/http` and `x-scheme-handler/https`.
- Intercept cross-profile link clicks inside Chrome via a companion Manifest V3 extension talking to the same binary over Chrome Native Messaging.
- Default to opening the configured fallback browser unless the user explicitly adds a rule or enables a handler manifest (no silent link hijacking).
- Keep the direct-open hot path headless and fast (do not initialize GTK unless the chooser popup or manager window is needed).
- Note on Wayland `XDG_ACTIVATION_TOKEN`: because this prototype links `libgtk-4` into the single binary, GTK's shared-library constructor clears `XDG_ACTIVATION_TOKEN` before `main()` runs. Splitting the headless router and the GTK4 UI into two binaries in a Cargo workspace is the clean follow-up when Wayland activation-token passthrough is needed.

## Architecture

One Rust binary (`link-router`) provides five entry points sharing one engine and one TOML configuration under `$XDG_CONFIG_HOME/link-router/`:

1. `link-router open [--ask] <url>`: OS-level handler invoked by `link-router.desktop`.
2. `link-router native-host`: Chrome Native Messaging host speaking 4-byte length-prefixed JSON over stdin/stdout.
3. `link-router manager` (default with no args): GTK4/libadwaita settings and rule manager.
4. `link-router test [--source <target>] [--ask] <url>`: Dry-run trace of URL unwrapping, rule matching, and target selection.
5. `link-router install` / `uninstall` / `status`: Installs `link-router.desktop`, optional `xdg-mime` default registration, and Chrome's Native Messaging host manifest.

## Resolution Order

1. Optional redirect unwrapping (Google `/url?q=`, Outlook SafeLinks, Slack, Facebook/Messenger, LinkedIn) up to 3 hops.
2. Optional tracking parameter stripping (`utm_*`, `fbclid`, `gclid`, `mc_cid`, `mc_eid`, `igshid`, `si`).
3. Match enabled user rules in `config.toml`, ranked by specificity:
   - Exact host > wildcard host (`*.example.com`) > catch-all (`*`).
   - Path pattern (`path_glob` / `path_prefix`, longer wins) > host-only.
   - Earlier rule order breaks ties.
4. Rule action:
   - `always`: open the configured target directly (or allow in place if `source_target` already matches it).
   - `ask`: show the GTK4 chooser popup with "Just once" and "Always" actions.
   - `browser`: open the configured fallback browser directly, bypassing any handler manifests.
5. If no user rule matches, check **enabled** drop-in manifests in `handlers.d/*.toml`:
   - 0 matches: open the fallback browser.
   - 1 match: open that handler's target.
   - 2+ matches: show the chooser popup with the matching candidates.
6. Passing `--ask` forces the chooser popup even when an `always` rule matches.

## Target Identifiers

- `chrome:<ProfileDir>`: launches `google-chrome --profile-directory=<ProfileDir> <url>`.
- `chrome-incognito:<ProfileDir>`: launches `google-chrome --incognito --profile-directory=<ProfileDir> <url>`.
- `firefox:<ProfileName>`: launches `firefox -P <ProfileName> <url>`.
- `desktop:<desktop-file-id>`: launches the `.desktop` application directly (never `link-router.desktop` or `xdg-open`).
- `custom:<id>`: launches a custom command from `config.toml` `[[targets]]`.

## Chrome Extension Interception

The Manifest V3 extension in `extension/` keeps a local cache of active rules fetched from `link-router native-host`:

- `content.js` intercepts left-clicks on matching `<a href>` elements with `preventDefault()` before the tab navigates, so the current tab stays intact while the target Chrome profile opens the link. Holding `Alt` bypasses interception once.
- `background.js` handles `webNavigation.onBeforeNavigate` (main frame only, `frameId === 0`) as a fallback for `file://` pages, `window.open`, redirects, and address-bar navigations. When redirected, existing tabs stay on their previous page while blank or newly opened tabs close.
- If the source Chrome profile already matches the resolved target, `link-router` returns `allow` and Chrome handles the navigation normally.
- URLs containing `link_router=browser` or any configured `bypass_query_params` entry bypass routing and open directly in the browser. Optional `detect_self_loop` avoids routing back into the app that invoked Link Router.
