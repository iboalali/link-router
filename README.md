# Link Router

Android-style URL and link router for Linux desktops and multi-profile Chrome setups.

## Build

Requires Rust, GTK 4 (`>= 4.14`), and libadwaita (`>= 1.6`).

```bash
cargo build --release
```

## Quick start

1. Check discovered targets and test a URL without changing system defaults:

```bash
./target/release/link-router targets
./target/release/link-router test https://acme.atlassian.net/browse/TM-123
```

2. Open the GTK4 manager to configure your fallback browser and rules:

```bash
./target/release/link-router manager
```

3. Example `~/.config/link-router/config.toml` for routing work Jira URLs to a specific Chrome profile:

```toml
fallback_target = "chrome:Default"
unwrap_redirects = true
strip_tracking_params = false
detect_self_loop = false
bypass_query_params = []

[[rules]]
id = "work-jira"
name = "Work Jira"
enabled = true
host = "*.atlassian.net"
action = "always"
target = "chrome:Profile 1"
```

- `link_router=browser` in a URL query string always bypasses routing and opens the link in the browser. Add custom query parameters or `key=value` pairs to `bypass_query_params` for apps that append their own bypass markers.
- Set `detect_self_loop = true` to detect when a target app invokes Link Router itself and fall back to the browser instead of looping.

4. Register as the desktop `http`/`https` handler:

```bash
./target/release/link-router install --register-default
```

To install the `.desktop` file and icon without changing your default browser yet:

```bash
./target/release/link-router install
```

## Chrome extension setup

1. Open `chrome://extensions` in each Chrome profile where you want links intercepted.
2. Enable **Developer mode** and click **Load unpacked**, selecting `extension/`.
3. Copy the unpacked extension ID and register the Native Messaging host:

```bash
./target/release/link-router install --extension-id <EXTENSION_ID>
```

4. Click the Link Router toolbar icon in each Chrome profile and select which profile that window belongs to (or let it auto-match by signed-in email).
5. Hold `Alt` while clicking a link to bypass routing once, or use the pause switch in the popup.

## Uninstall

```bash
./target/release/link-router uninstall
```

## License

MIT. See [LICENSE](LICENSE).
