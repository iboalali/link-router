use crate::config::{
    config_path, load_config, load_handler_manifests, save_config, slugify, Config,
    HandlerManifest, Rule, RuleAction,
};
use crate::engine::{resolve, Decision, ResolveOptions};
use crate::install;
use crate::targets::{discover_targets, TargetInfo};
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{self as gtk};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub fn run_manager() {
    let app = adw::Application::builder()
        .application_id("com.iboalali.LinkRouter.Manager")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(|app| {
        gtk::Window::set_default_icon_name(install::ICON_NAME);
        let state = Rc::new(RefCell::new(load_config().unwrap_or_default()));
        let manifests: Rc<Vec<HandlerManifest>> = Rc::new(load_handler_manifests());
        let targets: Rc<Vec<TargetInfo>> = Rc::new(discover_targets(&state.borrow()));

        let stack = adw::ViewStack::new();

        // --- Rules tab ---
        let rules_page = adw::PreferencesPage::new();
        let rules_group = adw::PreferencesGroup::builder()
            .title("URL Routing Rules")
            .description("Rules match by host and optional path prefix. Unmatched links open in your fallback browser.")
            .build();
        let add_rule_btn = gtk::Button::with_label("Add Rule");
        add_rule_btn.add_css_class("suggested-action");
        add_rule_btn.set_valign(gtk::Align::Center);
        rules_group.set_header_suffix(Some(&add_rule_btn));

        let rules_list = gtk::ListBox::new();
        rules_list.add_css_class("boxed-list");
        rules_list.set_selection_mode(gtk::SelectionMode::None);
        rules_group.add(&rules_list);
        rules_page.add(&rules_group);
        stack.add_titled_with_icon(&rules_page, Some("rules"), "Rules", "view-list-symbolic");

        // --- Apps tab ---
        let apps_page = adw::PreferencesPage::new();
        let handlers_group = adw::PreferencesGroup::builder()
            .title("App Handlers (handlers.d)")
            .description("Drop-in app manifests stay disabled until you turn them on here.")
            .build();
        let handlers_list = gtk::ListBox::new();
        handlers_list.add_css_class("boxed-list");
        handlers_list.set_selection_mode(gtk::SelectionMode::None);
        handlers_group.add(&handlers_list);
        apps_page.add(&handlers_group);

        let discovered_group = adw::PreferencesGroup::builder()
            .title("Discovered Targets")
            .description("Browser profiles, desktop apps, and custom commands available for rules.")
            .build();
        let discovered_list = gtk::ListBox::new();
        discovered_list.add_css_class("boxed-list");
        discovered_list.set_selection_mode(gtk::SelectionMode::None);
        for t in targets.iter() {
            let row = adw::ActionRow::builder()
                .title(&t.name)
                .subtitle(&format!("{} ({})", t.subtitle, t.id))
                .build();
            let icon = gtk::Image::from_icon_name(&t.icon_name);
            icon.set_pixel_size(24);
            row.add_prefix(&icon);
            discovered_list.append(&row);
        }
        discovered_group.add(&discovered_list);
        apps_page.add(&discovered_group);
        stack.add_titled_with_icon(
            &apps_page,
            Some("apps"),
            "Apps",
            "application-x-executable-symbolic",
        );

        // --- Test & Setup tab ---
        let setup_page = adw::PreferencesPage::new();

        let test_group = adw::PreferencesGroup::builder()
            .title("Test a URL")
            .description("Check how Link Router resolves a URL without opening a browser.")
            .build();
        let test_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let test_entry = gtk::Entry::builder()
            .placeholder_text("https://acme.atlassian.net/browse/TM-123")
            .hexpand(true)
            .build();
        let test_btn = gtk::Button::with_label("Test");
        test_btn.add_css_class("suggested-action");
        test_box.append(&test_entry);
        test_box.append(&test_btn);
        test_group.add(&test_box);

        let test_output = gtk::Label::new(Some("Enter a URL above to inspect its routing trace."));
        test_output.set_halign(gtk::Align::Start);
        test_output.set_wrap(true);
        test_output.set_margin_top(8);
        test_output.add_css_class("dim-label");
        test_group.add(&test_output);
        setup_page.add(&test_group);

        let general_group = adw::PreferencesGroup::builder()
            .title("General Settings")
            .build();

        let target_ids: Vec<String> = targets.iter().map(|t| t.id.clone()).collect();
        let target_labels: Vec<String> = targets
            .iter()
            .map(|t| format!("{} [{}]", t.name, t.id))
            .collect();
        let target_label_refs: Vec<&str> = target_labels.iter().map(|s| s.as_str()).collect();
        let fallback_row = adw::ComboRow::builder()
            .title("Fallback default browser")
            .subtitle("Used whenever no rule or enabled handler matches")
            .model(&gtk::StringList::new(&target_label_refs))
            .build();
        if let Some(idx) = target_ids
            .iter()
            .position(|id| id == &state.borrow().fallback_target)
        {
            fallback_row.set_selected(idx as u32);
        }
        {
            let st = state.clone();
            let ids = target_ids.clone();
            fallback_row.connect_selected_notify(move |row| {
                if let Some(id) = ids.get(row.selected() as usize) {
                    let mut cfg = st.borrow_mut();
                    cfg.fallback_target = id.clone();
                    let _ = save_config(&cfg);
                }
            });
        }
        general_group.add(&fallback_row);

        let unwrap_row = adw::SwitchRow::builder()
            .title("Unwrap redirect links")
            .subtitle("Extract target URLs from Google, Outlook SafeLinks, and Slack wrappers")
            .active(state.borrow().unwrap_redirects)
            .build();
        {
            let st = state.clone();
            unwrap_row.connect_active_notify(move |row| {
                let mut cfg = st.borrow_mut();
                cfg.unwrap_redirects = row.is_active();
                let _ = save_config(&cfg);
            });
        }
        general_group.add(&unwrap_row);

        let tracking_row = adw::SwitchRow::builder()
            .title("Strip tracking parameters")
            .subtitle("Remove utm_*, fbclid, gclid, and similar query parameters")
            .active(state.borrow().strip_tracking_params)
            .build();
        {
            let st = state.clone();
            tracking_row.connect_active_notify(move |row| {
                let mut cfg = st.borrow_mut();
                cfg.strip_tracking_params = row.is_active();
                let _ = save_config(&cfg);
            });
        }
        general_group.add(&tracking_row);

        let self_loop_row = adw::SwitchRow::builder()
            .title("Detect app self-loops")
            .subtitle("Open in fallback browser when a target app invokes Link Router for its own URL")
            .active(state.borrow().detect_self_loop)
            .build();
        {
            let st = state.clone();
            self_loop_row.connect_active_notify(move |row| {
                let mut cfg = st.borrow_mut();
                cfg.detect_self_loop = row.is_active();
                let _ = save_config(&cfg);
            });
        }
        general_group.add(&self_loop_row);
        setup_page.add(&general_group);

        let status = install::status();
        let status_group = adw::PreferencesGroup::builder()
            .title("Desktop and Chrome Integration")
            .build();
        status_group.add(
            &adw::ActionRow::builder()
                .title("Config file")
                .subtitle(glib::markup_escape_text(config_path().to_string_lossy().as_ref()))
                .build(),
        );
        status_group.add(
            &adw::ActionRow::builder()
                .title("Desktop entry")
                .subtitle(&format!(
                    "{} ({})",
                    if status.desktop_installed {
                        "Installed"
                    } else {
                        "Not installed"
                    },
                    status.desktop_file.display()
                ))
                .build(),
        );
        status_group.add(
            &adw::ActionRow::builder()
                .title("Default HTTP/HTTPS handler")
                .subtitle(&format!(
                    "http: {} | https: {}",
                    status.http_default.as_deref().unwrap_or("unset"),
                    status.https_default.as_deref().unwrap_or("unset")
                ))
                .build(),
        );
        status_group.add(
            &adw::ActionRow::builder()
                .title("Chrome Native Messaging host")
                .subtitle(&format!(
                    "{} ({})",
                    if status.native_host_installed {
                        "Installed"
                    } else {
                        "Not installed"
                    },
                    status.native_host_file.display()
                ))
                .build(),
        );
        setup_page.add(&status_group);

        stack.add_titled_with_icon(
            &setup_page,
            Some("setup"),
            "Test & Setup",
            "preferences-system-symbolic",
        );

        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&stack));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Link Router")
            .default_width(780)
            .default_height(560)
            .content(&toolbar_view)
            .build();

        // Populate handlers list
        populate_handlers_list(&handlers_list, &state, &manifests);

        // Populate rules list
        let refresh_rules: Rc<dyn Fn()> = {
            let list_ref = rules_list.clone();
            let win_ref = window.clone();
            let st_ref = state.clone();
            let targets_ref = targets.clone();
            Rc::new(move || {
                rebuild_rules_list(&list_ref, &win_ref, &st_ref, &targets_ref);
            })
        };
        refresh_rules();

        // Wire Test URL action
        {
            let entry_ref = test_entry.clone();
            let out_ref = test_output.clone();
            let st_ref = state.clone();
            let manifests_ref = manifests.clone();
            let run_test = Rc::new(move || {
                let raw = entry_ref.text().to_string();
                let cfg = st_ref.borrow();
                match resolve(&raw, &cfg, &manifests_ref, ResolveOptions::default()) {
                    Ok(res) => {
                        let summary = match &res.decision {
                            Decision::Open { target } => format!("Open directly in: {}", target),
                            Decision::AllowInSource { target } => {
                                format!("Allow in place: {}", target)
                            }
                            Decision::Ask { candidates, .. } => {
                                format!("Prompt chooser with: {}", candidates.join(", "))
                            }
                        };
                        let text = format!(
                            "{}\nEffective URL: {}\nTrace:\n  - {}",
                            summary,
                            res.effective_url,
                            res.trace.join("\n  - ")
                        );
                        out_ref.set_text(&text);
                    }
                    Err(e) => out_ref.set_text(&format!("Error: {}", e)),
                }
            });
            let r1 = run_test.clone();
            test_btn.connect_clicked(move |_| r1());
            let r2 = run_test.clone();
            test_entry.connect_activate(move |_| r2());
        }

        // Wire Add Rule dialog
        {
            let win_ref = window.clone();
            let st_ref = state.clone();
            let targets_ref = targets.clone();
            let refresh_ref = refresh_rules.clone();
            add_rule_btn.connect_clicked(move |_| {
                open_rule_dialog(&win_ref, &st_ref, &targets_ref, None, refresh_ref.clone());
            });
        }

        if let Ok(ms_str) = std::env::var("LINK_ROUTER_AUTO_CLOSE_MS") {
            if let Ok(ms) = ms_str.parse::<u64>() {
                let win_ref = window.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                    win_ref.close();
                });
            }
        }

        window.present();
    });

    app.run_with_args(&[] as &[&str]);
}

fn populate_handlers_list(
    list: &gtk::ListBox,
    state: &Rc<RefCell<Config>>,
    manifests: &[HandlerManifest],
) {
    if manifests.is_empty() {
        let empty = adw::ActionRow::builder()
            .title("No handler manifests found")
            .subtitle("Place *.toml files in ~/.config/link-router/handlers.d/ to register app link capabilities.")
            .build();
        list.append(&empty);
        return;
    }

    for manifest in manifests {
        let domains: Vec<String> = manifest.domains.iter().map(|d| d.host.clone()).collect();
        let row = adw::SwitchRow::builder()
            .title(&manifest.name)
            .subtitle(&format!("{} -> {}", domains.join(", "), manifest.target))
            .active(state.borrow().is_handler_enabled(&manifest.id))
            .build();
        let st = state.clone();
        let id = manifest.id.clone();
        row.connect_active_notify(move |r| {
            let mut cfg = st.borrow_mut();
            cfg.set_handler_enabled(&id, r.is_active());
            let _ = save_config(&cfg);
        });
        list.append(&row);
    }
}

fn rebuild_rules_list(
    list: &gtk::ListBox,
    parent: &adw::ApplicationWindow,
    state: &Rc<RefCell<Config>>,
    targets: &Rc<Vec<TargetInfo>>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let rules = state.borrow().rules.clone();
    if rules.is_empty() {
        let empty = adw::ActionRow::builder()
            .title("No rules configured yet")
            .subtitle("Click Add Rule to route a domain (such as *.atlassian.net) to a Chrome profile or app.")
            .build();
        list.append(&empty);
        return;
    }

    for (idx, rule) in rules.iter().enumerate() {
        let pattern = match (&rule.path_prefix, &rule.path_glob) {
            (Some(p), _) => format!("{}{}", rule.host, p),
            (None, Some(g)) => format!("{}{}", rule.host, g),
            (None, None) => rule.host.clone(),
        };
        let action_desc = match rule.action {
            RuleAction::Always => format!(
                "Always -> {}",
                rule.target.as_deref().unwrap_or("fallback")
            ),
            RuleAction::Ask => "Ask every time".to_string(),
            RuleAction::Browser => "Open in fallback browser".to_string(),
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&rule.name))
            .subtitle(glib::markup_escape_text(&format!(
                "{} | {}",
                pattern, action_desc
            )))
            .activatable(true)
            .build();

        let toggle = gtk::Switch::new();
        toggle.set_active(rule.enabled);
        toggle.set_valign(gtk::Align::Center);
        {
            let st = state.clone();
            toggle.connect_active_notify(move |sw| {
                let mut cfg = st.borrow_mut();
                if let Some(r) = cfg.rules.get_mut(idx) {
                    r.enabled = sw.is_active();
                    let _ = save_config(&cfg);
                }
            });
        }

        let open_edit: Rc<dyn Fn()> = {
            let win_ref = parent.clone();
            let st_ref = state.clone();
            let targets_ref = targets.clone();
            let list_ref = list.clone();
            Rc::new(move || {
                let list_cb = list_ref.clone();
                let win_cb = win_ref.clone();
                let st_cb = st_ref.clone();
                let targets_cb = targets_ref.clone();
                let on_saved: Rc<dyn Fn()> = Rc::new(move || {
                    rebuild_rules_list(&list_cb, &win_cb, &st_cb, &targets_cb);
                });
                open_rule_dialog(&win_ref, &st_ref, &targets_ref, Some(idx), on_saved);
            })
        };

        {
            let edit_cb = open_edit.clone();
            row.connect_activated(move |_| edit_cb());
        }

        let edit_btn = gtk::Button::from_icon_name("document-edit-symbolic");
        edit_btn.set_valign(gtk::Align::Center);
        edit_btn.add_css_class("flat");
        edit_btn.set_tooltip_text(Some("Edit rule"));
        {
            let edit_cb = open_edit.clone();
            edit_btn.connect_clicked(move |_| edit_cb());
        }

        let delete_btn = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_btn.set_valign(gtk::Align::Center);
        delete_btn.add_css_class("flat");
        delete_btn.set_tooltip_text(Some("Delete rule"));
        {
            let st = state.clone();
            let list_ref = list.clone();
            let win_ref = parent.clone();
            let targets_ref = targets.clone();
            delete_btn.connect_clicked(move |_| {
                {
                    let mut cfg = st.borrow_mut();
                    if idx < cfg.rules.len() {
                        cfg.rules.remove(idx);
                        let _ = save_config(&cfg);
                    }
                }
                rebuild_rules_list(&list_ref, &win_ref, &st, &targets_ref);
            });
        }

        row.add_suffix(&toggle);
        row.add_suffix(&edit_btn);
        row.add_suffix(&delete_btn);
        list.append(&row);
    }
}

fn open_rule_dialog(
    parent: &adw::ApplicationWindow,
    state: &Rc<RefCell<Config>>,
    targets: &[TargetInfo],
    edit_idx: Option<usize>,
    on_saved: Rc<dyn Fn()>,
) {
    let existing = edit_idx.and_then(|idx| state.borrow().rules.get(idx).cloned());
    let title = if existing.is_some() {
        "Edit Routing Rule"
    } else {
        "Add Routing Rule"
    };

    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(440)
        .build();

    let header = adw::HeaderBar::new();
    let box_root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_root.set_margin_top(12);
    box_root.set_margin_bottom(12);
    box_root.set_margin_start(16);
    box_root.set_margin_end(16);

    let name_entry = gtk::Entry::builder()
        .placeholder_text("Rule name (e.g. Work Jira)")
        .build();
    let host_entry = gtk::Entry::builder()
        .placeholder_text("Host pattern (e.g. *.atlassian.net or jira.company.de)")
        .build();
    let path_entry = gtk::Entry::builder()
        .placeholder_text("Optional path prefix (e.g. /browse/)")
        .build();

    let action_dd = gtk::DropDown::from_strings(&[
        "Always open target",
        "Ask every time",
        "Open in fallback browser",
    ]);

    let mut target_ids: Vec<String> = targets.iter().map(|t| t.id.clone()).collect();
    let mut target_labels: Vec<String> = targets
        .iter()
        .map(|t| format!("{} [{}]", t.name, t.id))
        .collect();

    if let Some(existing_target) = existing.as_ref().and_then(|r| r.target.as_ref()) {
        if !target_ids.iter().any(|id| id == existing_target) {
            target_ids.push(existing_target.clone());
            target_labels.push(format!("{} (unavailable)", existing_target));
        }
    }

    let target_label_refs: Vec<&str> = target_labels.iter().map(|s| s.as_str()).collect();
    let target_dd = gtk::DropDown::from_strings(&target_label_refs);

    if let Some(rule) = &existing {
        name_entry.set_text(&rule.name);
        host_entry.set_text(&rule.host);
        if let Some(prefix) = &rule.path_prefix {
            path_entry.set_text(prefix);
        }
        let action_idx = match rule.action {
            RuleAction::Always => 0,
            RuleAction::Ask => 1,
            RuleAction::Browser => 2,
        };
        action_dd.set_selected(action_idx);
        if let Some(target_id) = &rule.target {
            if let Some(pos) = target_ids.iter().position(|id| id == target_id) {
                target_dd.set_selected(pos as u32);
            }
        }
    } else {
        action_dd.set_selected(0);
        target_dd.set_selected(0);
    }

    target_dd.set_sensitive(action_dd.selected() != 2);
    {
        let target_dd_ref = target_dd.clone();
        action_dd.connect_selected_notify(move |dd| {
            target_dd_ref.set_sensitive(dd.selected() != 2);
        });
    }

    box_root.append(&name_entry);
    box_root.append(&host_entry);
    box_root.append(&path_entry);
    box_root.append(&action_dd);
    box_root.append(&target_dd);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel_btn = gtk::Button::with_label("Cancel");
    let save_btn = gtk::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    buttons.append(&cancel_btn);
    buttons.append(&save_btn);
    box_root.append(&buttons);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&box_root));
    dialog.set_content(Some(&tv));

    {
        let dlg = dialog.clone();
        cancel_btn.connect_clicked(move |_| dlg.close());
    }
    {
        let dlg = dialog.clone();
        let st = state.clone();
        let ids = target_ids;
        save_btn.connect_clicked(move |_| {
            let host = host_entry.text().trim().to_string();
            if host.is_empty() {
                return;
            }
            let name = {
                let n = name_entry.text().trim().to_string();
                if n.is_empty() {
                    host.clone()
                } else {
                    n
                }
            };
            let path_prefix = {
                let p = path_entry.text().trim().to_string();
                if p.is_empty() {
                    None
                } else {
                    Some(p)
                }
            };
            let action = match action_dd.selected() {
                1 => RuleAction::Ask,
                2 => RuleAction::Browser,
                _ => RuleAction::Always,
            };
            let target = match action {
                RuleAction::Browser => None,
                RuleAction::Always | RuleAction::Ask => {
                    ids.get(target_dd.selected() as usize).cloned()
                }
            };

            {
                let mut cfg = st.borrow_mut();
                if let Some(rule) = edit_idx.and_then(|idx| cfg.rules.get_mut(idx)) {
                    rule.name = name;
                    rule.host = host;
                    rule.path_prefix = path_prefix;
                    rule.action = action;
                    rule.target = target;
                } else {
                    let next_idx = cfg.rules.len();
                    cfg.rules.push(Rule {
                        id: slugify(&format!("{}-{}", name, next_idx)),
                        name,
                        enabled: true,
                        host,
                        path_prefix,
                        path_glob: None,
                        action,
                        target,
                        candidates: Vec::new(),
                    });
                }
                let _ = save_config(&cfg);
            }
            on_saved();
            dlg.close();
        });
    }

    dialog.present();
}
