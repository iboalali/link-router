use crate::targets::TargetInfo;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{self as gtk};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveScope {
    HostOnly,
    HostAndPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserOutcome {
    pub target_id: String,
    pub save_scope: Option<SaveScope>,
}

pub fn run_chooser(
    url: &str,
    host: &str,
    suggested_path_prefix: Option<&str>,
    candidate_ids: &[String],
    all_targets: &[TargetInfo],
) -> Option<ChooserOutcome> {
    let result: Rc<RefCell<Option<ChooserOutcome>>> = Rc::new(RefCell::new(None));
    let url_owned = url.to_string();
    let host_owned = host.to_string();
    let prefix_owned = suggested_path_prefix.map(|s| s.to_string());
    let candidate_ids_owned = candidate_ids.to_vec();
    let all_targets_owned = all_targets.to_vec();

    let app = adw::Application::builder()
        .application_id("com.iboalali.LinkRouter.Chooser")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let result_clone = result.clone();
    app.connect_activate(move |app| {
        gtk::Window::set_default_icon_name(crate::install::ICON_NAME);
        let mut primary: Vec<TargetInfo> = Vec::new();
        for cid in &candidate_ids_owned {
            if let Some(info) = all_targets_owned.iter().find(|t| &t.id == cid) {
                if !primary.iter().any(|p| p.id == info.id) {
                    primary.push(info.clone());
                }
            } else {
                primary.push(TargetInfo {
                    id: cid.clone(),
                    name: cid.clone(),
                    subtitle: "Configured target".to_string(),
                    icon_name: "web-browser".to_string(),
                    kind: crate::targets::TargetKind::Custom,
                    email: None,
                    profile_dir: None,
                });
            }
        }
        if primary.is_empty() {
            primary = all_targets_owned.clone();
        }
        let others: Vec<TargetInfo> = all_targets_owned
            .iter()
            .filter(|t| !primary.iter().any(|p| p.id == t.id))
            .cloned()
            .collect();

        let selected_target: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(
            primary.first().map(|t| t.id.clone()),
        ));

        let header = adw::HeaderBar::new();
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.set_tooltip_text(Some("Copy URL"));
        header.pack_end(&copy_btn);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(16);
        root.set_margin_end(16);

        let url_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let host_label = gtk::Label::new(Some(&host_owned));
        host_label.set_halign(gtk::Align::Start);
        host_label.add_css_class("title-3");
        let url_label = gtk::Label::new(Some(&url_owned));
        url_label.set_halign(gtk::Align::Start);
        url_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        url_label.set_max_width_chars(56);
        url_label.add_css_class("dim-label");
        url_label.add_css_class("caption");
        url_box.append(&host_label);
        url_box.append(&url_label);
        root.append(&url_box);

        let primary_list = gtk::ListBox::new();
        primary_list.add_css_class("boxed-list");
        primary_list.set_selection_mode(gtk::SelectionMode::Single);

        let primary_ids: Rc<Vec<String>> = Rc::new(primary.iter().map(|t| t.id.clone()).collect());
        for t in &primary {
            primary_list.append(&build_target_row(t));
        }
        if let Some(first_row) = primary_list.row_at_index(0) {
            primary_list.select_row(Some(&first_row));
        }
        root.append(&primary_list);

        let other_list = gtk::ListBox::new();
        other_list.add_css_class("boxed-list");
        other_list.set_selection_mode(gtk::SelectionMode::Single);
        let other_ids: Rc<Vec<String>> = Rc::new(others.iter().map(|t| t.id.clone()).collect());
        if !others.is_empty() {
            for t in &others {
                other_list.append(&build_target_row(t));
            }
            let scroll = gtk::ScrolledWindow::builder()
                .max_content_height(180)
                .propagate_natural_height(true)
                .child(&other_list)
                .build();
            let expander = gtk::Expander::new(Some("Other apps and browsers"));
            expander.set_child(Some(&scroll));
            root.append(&expander);
        }

        let scope_options: Vec<String> = match &prefix_owned {
            Some(prefix) => vec![
                format!("Host only ({})", host_owned),
                format!("Host and path ({}{})", host_owned, prefix),
            ],
            None => vec![format!("Host only ({})", host_owned)],
        };
        let scope_strs: Vec<&str> = scope_options.iter().map(|s| s.as_str()).collect();
        let scope_dropdown = gtk::DropDown::from_strings(&scope_strs);
        scope_dropdown.set_selected(0);

        let scope_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let scope_lbl = gtk::Label::new(Some("Always rule scope:"));
        scope_lbl.add_css_class("dim-label");
        scope_row.append(&scope_lbl);
        scope_dropdown.set_hexpand(true);
        scope_row.append(&scope_dropdown);
        root.append(&scope_row);

        let button_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        button_box.set_halign(gtk::Align::End);
        button_box.set_margin_top(6);

        let cancel_btn = gtk::Button::with_label("Cancel");
        let once_btn = gtk::Button::with_label("Just once");
        let always_btn = gtk::Button::with_label("Always");
        always_btn.add_css_class("suggested-action");

        button_box.append(&cancel_btn);
        button_box.append(&once_btn);
        button_box.append(&always_btn);
        root.append(&button_box);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&root));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Open Link With")
            .default_width(460)
            .resizable(false)
            .content(&toolbar_view)
            .build();

        {
            let url_copy = url_owned.clone();
            let win_copy = window.clone();
            copy_btn.connect_clicked(move |_| {
                win_copy.clipboard().set_text(&url_copy);
            });
        }

        {
            let sel = selected_target.clone();
            let ids = primary_ids.clone();
            let other_ref = other_list.clone();
            primary_list.connect_row_selected(move |_, row| {
                if let Some(r) = row {
                    other_ref.unselect_all();
                    let idx = r.index() as usize;
                    if let Some(id) = ids.get(idx) {
                        *sel.borrow_mut() = Some(id.clone());
                    }
                }
            });
        }
        {
            let sel = selected_target.clone();
            let ids = other_ids.clone();
            let primary_ref = primary_list.clone();
            other_list.connect_row_selected(move |_, row| {
                if let Some(r) = row {
                    primary_ref.unselect_all();
                    let idx = r.index() as usize;
                    if let Some(id) = ids.get(idx) {
                        *sel.borrow_mut() = Some(id.clone());
                    }
                }
            });
        }

        let finish = {
            let result_ref = result_clone.clone();
            let sel_ref = selected_target.clone();
            let scope_ref = scope_dropdown.clone();
            let win_ref = window.clone();
            let has_prefix = prefix_owned.is_some();
            Rc::new(move |save: bool| {
                if let Some(target_id) = sel_ref.borrow().clone() {
                    let save_scope = if save {
                        if has_prefix && scope_ref.selected() == 1 {
                            Some(SaveScope::HostAndPath)
                        } else {
                            Some(SaveScope::HostOnly)
                        }
                    } else {
                        None
                    };
                    *result_ref.borrow_mut() = Some(ChooserOutcome {
                        target_id,
                        save_scope,
                    });
                }
                win_ref.close();
            })
        };

        {
            let win_ref = window.clone();
            cancel_btn.connect_clicked(move |_| win_ref.close());
        }
        {
            let f = finish.clone();
            once_btn.connect_clicked(move |_| f(false));
        }
        {
            let f = finish.clone();
            always_btn.connect_clicked(move |_| f(true));
        }
        {
            let f = finish.clone();
            primary_list.connect_row_activated(move |_, _| f(false));
        }
        {
            let f = finish.clone();
            other_list.connect_row_activated(move |_, _| f(false));
        }

        let key_ctrl = gtk::EventControllerKey::new();
        {
            let f = finish.clone();
            let win_ref = window.clone();
            key_ctrl.connect_key_pressed(move |_, key, _, mods| {
                if key == gdk::Key::Escape {
                    win_ref.close();
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
                    let save = mods.contains(gdk::ModifierType::SHIFT_MASK);
                    f(save);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(key_ctrl);

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
    let final_outcome = result.borrow().clone();
    final_outcome
}

fn build_target_row(target: &TargetInfo) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&target.name))
        .subtitle(glib::markup_escape_text(&target.subtitle))
        .activatable(true)
        .build();
    let icon = gtk::Image::from_icon_name(&target.icon_name);
    icon.set_pixel_size(24);
    row.add_prefix(&icon);
    row
}
