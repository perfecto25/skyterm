//! Regression tests for the GUI rules we've settled on (menu content/size/
//! colour, pane + tab behaviour, focus dimming, highlight colour, close
//! confirmation, About window, config persistence).
//!
//! Two kinds:
//!
//! * **Pure tests** — no display needed. They check the CSS text, the menu
//!   model, and the small decision functions (`dim_scrim`,
//!   `close_confirmation`, `normalize_hex_color`).
//! * **`gui_end_to_end`** — boots the *real* application (`on_activate`) and
//!   drives it. GTK has to live on a single thread, so every GUI check runs
//!   sequentially inside this one `#[test]`. It needs a display (use
//!   `xvfb-run -a cargo test` headless) and is skipped with a notice when
//!   there isn't one. It points `XDG_CONFIG_HOME` / `XDG_DATA_HOME` at a temp
//!   dir and disables the session bus so it can never touch the user's real
//!   config or their running skyterm.

use super::*;
use std::time::{Duration, Instant};

// ───────────────────────────── pure tests ─────────────────────────────

/// The declaration block of the rule whose selector list is exactly
/// `selector` (matched at the start of a line), e.g. `.pane-wrap.focused`.
fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
    let needle = format!("{selector} {{");
    let start = css
        .match_indices(&needle)
        .map(|(i, _)| i)
        .find(|&i| i == 0 || css.as_bytes()[i - 1] == b'\n')
        .unwrap_or_else(|| panic!("no CSS rule with selector `{selector}`"));
    let body = &css[start + needle.len()..];
    &body[..body.find('}').expect("unterminated rule")]
}

fn has_decl(block: &str, decl: &str) -> bool {
    block.split(';').any(|d| {
        let norm: String = d.split_whitespace().collect::<Vec<_>>().join(" ");
        norm == decl
    })
}

#[test]
fn css_never_uses_important() {
    // GTK4's `!important` cascade is inverted: it makes our rules LOSE to the
    // theme (see CLAUDE.md, "dark menu").
    assert!(!CSS_DARK_MENU.contains("!important"));
    assert!(!CSS_TEMPLATE.contains("!important"));
}

#[test]
fn css_placeholders_are_substituted() {
    let css = build_css("Some Font", "AABBCC", 5);
    assert!(css.contains("#AABBCC"), "highlight colour not substituted");
    assert!(css.contains("\"Some Font\""), "font family not substituted");
    assert!(!css.contains("__HIGHLIGHT__"));
    assert!(!css.contains("__BORDER__"));
    assert!(!css.contains("__FONT_FAMILY__"));
}

#[test]
fn focused_pane_border_uses_configured_size_and_colour() {
    let wrap = rule(CSS_TEMPLATE, ".pane-wrap");
    assert!(has_decl(wrap, "border: __BORDER__px solid transparent"), "{wrap}");
    let focused = rule(CSS_TEMPLATE, ".pane-wrap.focused");
    assert!(has_decl(focused, "border-color: #__HIGHLIGHT__"), "{focused}");
    // The configured size lands in the stylesheet…
    let css = build_css("F", "FF3399", 9);
    assert!(rule(&css, ".pane-wrap").contains("border: 9px solid transparent"));
    // …and a zero-width border is what "highlight off" produces.
    let off = build_css("F", "FF3399", highlight_border_px(false, 9));
    assert!(rule(&off, ".pane-wrap").contains("border: 0px solid transparent"), "{off}");
}

#[test]
fn highlight_border_size_rules() {
    assert_eq!(DEFAULT_BORDER_PX, 2, "default border is 2px");
    assert_eq!(highlight_border_px(true, 5), 5);
    assert_eq!(highlight_border_px(true, 12), 12);
    assert_eq!(highlight_border_px(false, 12), 0, "no border when the highlight is off");
    // Out-of-range config values are clamped, never 0/huge.
    assert_eq!(clamp_border_px(0), MIN_BORDER_PX);
    assert_eq!(clamp_border_px(1_000), MAX_BORDER_PX);
    assert_eq!(highlight_border_px(true, 0), MIN_BORDER_PX);
}

#[test]
fn default_highlight_colour_is_ff3399() {
    assert_eq!(DEFAULT_HIGHLIGHT, "FF3399");
    assert_eq!(normalize_hex_color(DEFAULT_HIGHLIGHT).as_deref(), Some("FF3399"));
}

#[test]
fn hex_colour_parsing() {
    assert_eq!(normalize_hex_color("ff00cc").as_deref(), Some("FF00CC"));
    assert_eq!(normalize_hex_color("#66cc33").as_deref(), Some("66CC33"));
    assert_eq!(normalize_hex_color("  FF3399 ").as_deref(), Some("FF3399"));
    for bad in ["", "FFF", "FF339", "FF33999", "GG0000", "#12345Z", "FF 339"] {
        assert_eq!(normalize_hex_color(bad), None, "{bad:?} should be rejected");
    }
}

#[test]
fn menu_css_is_dark_rounded_and_borderless() {
    let contents = rule(CSS_DARK_MENU, "popover.skyterm-menu > contents");
    assert!(has_decl(contents, "border: none"), "no white border: {contents}");
    assert!(has_decl(contents, "border-radius: 12px"), "{contents}");
    // The popover node itself must not paint a square dark backdrop.
    let popover = rule(CSS_DARK_MENU, "popover.skyterm-menu");
    assert!(has_decl(popover, "background: transparent"), "{popover}");
    // Dark background + light text on the contents.
    let bg = CSS_DARK_MENU
        .split("popover.skyterm-menu > contents,")
        .nth(1)
        .expect("contents background rule");
    assert!(bg.contains("background-color: #1e1e1e"));
    assert!(bg.contains("color:            #e0e0e0"));
}

#[test]
fn menu_item_tints_and_close_button_colours() {
    assert!(has_decl(rule(CSS_DARK_MENU, "popover.skyterm-menu label.menu-tab"), "color: #2ecc71"));
    assert!(has_decl(rule(CSS_DARK_MENU, "popover.skyterm-menu label.menu-window"), "color: #ff79c6"));
    assert!(has_decl(rule(CSS_TEMPLATE, "button.pane-close-btn"), "color: #f1c40f"));
}

#[test]
fn menu_font_follows_terminal_font() {
    let block = rule(
        CSS_TEMPLATE,
        "popover.menu,\npopover.menu label,\npopover.menu modelbutton,\npopover.menu modelbutton accelerator",
    );
    assert!(has_decl(block, "font-family: __FONT_FAMILY__, monospace"), "{block}");
}

#[test]
fn about_card_is_rounded_with_own_shadow() {
    let card = rule(CSS_TEMPLATE, ".skyterm-about-card");
    assert!(has_decl(card, "border-radius: 14px"), "{card}");
    assert!(card.contains("box-shadow"), "{card}");
    // Undecorated + transparent window so the corners aren't the WM's.
    let win = rule(
        CSS_TEMPLATE,
        "window.skyterm-about,\nwindow.skyterm-about.csd,\nwindow.skyterm-about.solid-csd",
    );
    assert!(has_decl(win, "background: transparent"), "{win}");
}

#[test]
fn metrics_label_uses_terminal_font_and_is_compact() {
    let r = rule(CSS_TEMPLATE, ".skyterm-metrics");
    assert!(has_decl(r, "font-family: __FONT_FAMILY__, monospace"), "{r}");
    assert!(has_decl(r, "font-size: 12px"), "{r}");
}

#[test]
fn title_is_large_and_bold() {
    let r = rule(CSS_TEMPLATE, ".skyterm-title");
    assert!(has_decl(r, "font-size: 16px"), "{r}");
    assert!(has_decl(r, "font-weight: bold"), "{r}");
}

#[test]
fn dim_scrim_rules() {
    // Focused pane never dims.
    assert_eq!(dim_scrim(true, 5, true), None);
    // Setting off → never dims.
    assert_eq!(dim_scrim(false, 5, false), None);
    // 0% means no scrim at all.
    assert_eq!(dim_scrim(true, 0, false), None);
    // Unfocused + on → the configured strength.
    assert_eq!(dim_scrim(true, 5, false), Some(0.05));
    assert_eq!(dim_scrim(true, 20, false), Some(0.20));
    // Clamped to the maximum.
    assert_eq!(dim_scrim(true, 500, false), Some(MAX_DIM_PERCENT as f64 / 100.0));
}

#[test]
fn close_confirmation_rules() {
    // Only pane of only tab: always asks, even with the setting off.
    assert_eq!(close_confirmation(1, 1, false), CloseConfirm::LastPane);
    assert_eq!(close_confirmation(1, 1, true), CloseConfirm::LastPane);
    // Otherwise it follows the setting.
    assert_eq!(close_confirmation(1, 2, true), CloseConfirm::Pane);
    assert_eq!(close_confirmation(1, 2, false), CloseConfirm::None);
    assert_eq!(close_confirmation(3, 1, true), CloseConfirm::Pane);
    assert_eq!(close_confirmation(3, 1, false), CloseConfirm::None);
}

/// Labels of the items in section `i` of a menu model.
fn section_labels(menu: &gio::MenuModel, i: i32) -> Vec<String> {
    let section = menu.item_link(i, "section").expect("section link");
    (0..section.n_items())
        .map(|j| {
            section
                .item_attribute_value(j, "label", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn menu_model_layout() {
    let menu = build_menu_model(&Theme::presets(), &Default::default());
    let m = menu.upcast_ref::<gio::MenuModel>();
    assert_eq!(m.n_items(), 5, "five sections");
    assert_eq!(
        section_labels(m, 0),
        ["― Split Horizontally", "| Split Vertically", "⛶ Focus Pane", "Layouts", "New Tab", "New Window"]
    );
    assert_eq!(section_labels(m, 1), ["Maximize Window", "Minimize Window"]);
    assert_eq!(section_labels(m, 2), ["Copy", "Paste", "Select All"]);
    assert_eq!(section_labels(m, 3), ["Themes", "Shortcuts…", "Settings…", "About…"]);
}

#[test]
fn menu_always_ends_with_close_pane_slot() {
    // Regression: Close pane used to be dropped for a lone pane and then
    // clipped after a split. The menu must always carry it, as the last
    // section, as the custom "close-pane" widget slot.
    let menu = build_menu_model(&Theme::presets(), &Default::default());
    let m = menu.upcast_ref::<gio::MenuModel>();
    let last = m.item_link(m.n_items() - 1, "section").expect("last section");
    assert_eq!(last.n_items(), 1);
    let custom = last
        .item_attribute_value(0, "custom", Some(glib::VariantTy::STRING))
        .and_then(|v| v.get::<String>());
    assert_eq!(custom.as_deref(), Some("close-pane"));
}

#[test]
fn every_menu_action_is_registered() {
    let known: std::collections::HashSet<String> = Action::all()
        .iter()
        .map(|a| a.id().to_string())
        .chain(["layout", "settings", "about", "set-theme"].map(String::from))
        .collect();
    fn walk(m: &gio::MenuModel, known: &std::collections::HashSet<String>, out: &mut Vec<String>) {
        for i in 0..m.n_items() {
            if let Some(a) = m
                .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
            {
                let id = a.strip_prefix("pane.").unwrap_or_else(|| panic!("{a} not in pane group"));
                if !known.contains(id) {
                    out.push(a);
                }
            }
            for link in ["section", "submenu"] {
                if let Some(sub) = m.item_link(i, link) {
                    walk(&sub, known, out);
                }
            }
        }
    }
    let menu = build_menu_model(&Theme::presets(), &Default::default());
    let mut missing = Vec::new();
    walk(menu.upcast_ref(), &known, &mut missing);
    assert!(missing.is_empty(), "menu items with no registered action: {missing:?}");
}

#[test]
fn menu_has_layouts_and_every_theme() {
    let themes = Theme::presets();
    let menu = build_menu_model(&themes, &Default::default());
    let mut labels = Vec::new();
    fn collect(m: &gio::MenuModel, out: &mut Vec<String>) {
        for i in 0..m.n_items() {
            if let Some(l) = m
                .item_attribute_value(i, "label", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
            {
                out.push(l);
            }
            for link in ["section", "submenu"] {
                if let Some(sub) = m.item_link(i, link) {
                    collect(&sub, out);
                }
            }
        }
    }
    collect(menu.upcast_ref(), &mut labels);
    for t in &themes {
        assert!(labels.contains(&t.name), "theme {} missing from menu", t.name);
    }
    for (id, label) in LAYOUT_CHOICES.iter().filter(|(id, _)| *id != "single") {
        assert!(labels.iter().any(|l| l == label), "layout {id} missing");
    }
}

#[test]
fn layout_ids_round_trip() {
    for (id, _) in LAYOUT_CHOICES {
        let _ = Layout::from_id(id);
    }
    assert_eq!(Layout::from_id("2h"), Layout::TwoH);
    assert_eq!(Layout::from_id("4"), Layout::Four);
    assert_eq!(Layout::from_id("nonsense"), Layout::Single);
}

#[test]
fn menu_minimum_width_fits_longest_theme_name() {
    // ~14 chars at the terminal font plus padding.
    assert!(MENU_MIN_WIDTH >= 240);
}

// ─────────────────────── end-to-end GUI test ───────────────────────

/// Run the GLib main loop for `ms` milliseconds.
fn pump(ms: u64) {
    let ctx = glib::MainContext::default();
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        while ctx.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn find_widget(root: &gtk4::Widget, pred: &dyn Fn(&gtk4::Widget) -> bool) -> Option<gtk4::Widget> {
    if pred(root) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(found) = find_widget(&c, pred) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn find_all(root: &gtk4::Widget, pred: &dyn Fn(&gtk4::Widget) -> bool, out: &mut Vec<gtk4::Widget>) {
    if pred(root) {
        out.push(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        find_all(&c, pred, out);
        child = c.next_sibling();
    }
}

fn find_label(root: &gtk4::Widget, text: &str) -> Option<gtk4::Label> {
    find_widget(root, &|w| {
        w.downcast_ref::<gtk4::Label>().is_some_and(|l| l.text() == text)
    })
    .and_then(|w| w.downcast::<gtk4::Label>().ok())
}

fn rgb(c: gdk::RGBA) -> (u8, u8, u8) {
    let f = |v: f32| (v * 255.0).round() as u8;
    (f(c.red()), f(c.green()), f(c.blue()))
}

fn cur_tab(st: &Rc<WindowState>) -> Rc<Tab> {
    current_tab(st).expect("a current tab")
}

fn panes(st: &Rc<WindowState>) -> Vec<Rc<Pane>> {
    cur_tab(st).panes.borrow().clone()
}

/// Pop the pane menu at `at` (pane-local coordinates), let it settle, and
/// return the popover.
fn open_menu(st: &Rc<WindowState>, pane: &Rc<Pane>, at: (f64, f64)) -> PopoverMenu {
    show_pane_menu(st, pane, Some(at));
    pump(900);
    pane.menu_popover.borrow().clone().expect("pane has a menu popover")
}

fn dismiss(p: &PopoverMenu) {
    p.popdown();
    pump(250);
}

/// The invariants every popup of the right-click menu must satisfy.
fn check_menu(ctx: &str, st: &Rc<WindowState>, pane: &Rc<Pane>, p: &PopoverMenu, at: (f64, f64)) {
    let root: gtk4::Widget = p.clone().upcast();
    eprintln!(
        "[{ctx}] window {}x{}, menu {}x{} at {:?}",
        st.window.width(), st.window.height(), p.width(), p.height(), at
    );
    assert!(p.is_mapped(), "{ctx}: menu not shown");
    assert_eq!(
        p.parent().as_ref(),
        Some(st.window.upcast_ref::<gtk4::Widget>()),
        "{ctx}: popover must be parented to the toplevel window"
    );
    assert!(p.width() >= MENU_MIN_WIDTH, "{ctx}: menu narrower than {MENU_MIN_WIDTH}px: {}", p.width());
    assert!(!p.has_arrow(), "{ctx}: menu should have no arrow");

    // Placement: pointing at the click, translated into window coordinates.
    let pt = gtk4::graphene::Point::new(at.0 as f32, at.1 as f32);
    let win_pt = pane.gl_area.compute_point(&st.window, &pt).expect("compute_point");
    let (has_rect, rect) = p.pointing_to();
    assert!(has_rect, "{ctx}: popover has no pointing rect");
    assert_eq!((rect.x(), rect.y()), (win_pt.x() as i32, win_pt.y() as i32), "{ctx}: menu placed away from the click");

    // Close pane is ALWAYS there, visible, and fully inside the popover.
    let close = find_widget(&root, &|w| w.has_css_class("pane-close-btn"))
        .unwrap_or_else(|| panic!("{ctx}: Close pane button missing"));
    assert!(close.is_mapped(), "{ctx}: Close pane not mapped");
    assert!(close.height() > 0, "{ctx}: Close pane has no height");
    let b = close.compute_bounds(&root).expect("close bounds");
    assert!(b.y() >= 0.0, "{ctx}: Close pane above the menu: {b:?}");
    assert!(
        b.y() + b.height() <= p.height() as f32 + 1.0,
        "{ctx}: Close pane clipped: bottom {} > menu height {}",
        b.y() + b.height(),
        p.height()
    );

    // No visible scroller may need to scroll (that's how items got clipped).
    let mut sws = Vec::new();
    find_all(&root, &|w| w.is::<gtk4::ScrolledWindow>() && w.is_mapped(), &mut sws);
    for sw in sws {
        let sw = sw.downcast::<gtk4::ScrolledWindow>().unwrap();
        let adj = sw.vadjustment();
        assert!(
            adj.upper() - adj.page_size() < 1.5,
            "{ctx}: menu scrolls (content {} > view {}): items are clipped",
            adj.upper(),
            adj.page_size()
        );
    }

    // Every top-level menu entry is on screen and in order.
    for label in ["Split Horizontally", "Split Vertically", "Focus Pane", "New Tab", "New Window",
                  "Copy", "Paste", "Select All", "Settings…", "About…"] {
        let l = find_widget(&root, &|w| {
            w.downcast_ref::<gtk4::Label>().is_some_and(|l| l.text().ends_with(label))
        })
        .unwrap_or_else(|| panic!("{ctx}: menu item {label:?} missing"));
        assert!(l.is_mapped(), "{ctx}: {label:?} not mapped");
    }

    // Colours (the dark menu CSS actually won the cascade).
    let copy = find_label(&root, "Copy").expect("Copy label");
    assert_eq!(rgb(copy.color()), (0xe0, 0xe0, 0xe0), "{ctx}: menu text colour");
    let new_tab = find_label(&root, "New Tab").expect("New Tab label");
    assert_eq!(rgb(new_tab.color()), (0x2e, 0xcc, 0x71), "{ctx}: New Tab should be green");
    let new_win = find_label(&root, "New Window").expect("New Window label");
    assert_eq!(rgb(new_win.color()), (0xff, 0x79, 0xc6), "{ctx}: New Window should be pink");
    assert_eq!(rgb(close.color()), (0xf1, 0xc4, 0x0f), "{ctx}: Close pane colour");

    // Font follows the terminal font.
    let family = copy.pango_context().font_description().and_then(|d| d.family().map(|f| f.to_string()));
    assert!(
        family.as_deref().is_some_and(|f| f.contains("JetBrains Mono")),
        "{ctx}: menu font is {family:?}, expected the terminal font"
    );
}

fn toplevel_count() -> usize {
    gtk4::Window::list_toplevels().len()
}

// GTK on macOS must run on the process's main thread, which libtest's worker
// threads aren't, so this windowed test is Linux-only for now.
#[test]
#[cfg_attr(target_os = "macos", ignore = "GTK needs the main thread on macOS")]
fn gui_end_to_end() {
    // Never touch the user's real config / data / running instance.
    let tmp = std::env::temp_dir().join(format!("skyterm-uitest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // SAFETY: set before any other thread in this test reads the environment.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
        std::env::set_var("XDG_DATA_HOME", tmp.join("data"));
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent");
        std::env::set_var("SHELL", "/bin/sh");
    }

    if std::env::var_os("SKYTERM_SKIP_GUI_TESTS").is_some() {
        eprintln!("SKIPPED gui_end_to_end: SKYTERM_SKIP_GUI_TESTS is set");
        return;
    }
    if gtk4::init().is_err() {
        eprintln!("SKIPPED gui_end_to_end: no display (run with `xvfb-run -a cargo test`)");
        return;
    }

    let app = gtk4::Application::builder()
        .application_id("dev.skyterm.UiTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(on_activate);
    let _ = app.register(None::<&gio::Cancellable>);
    app.activate();
    pump(1200);
    let st = TEST_STATE
        .with(|t| t.borrow().clone())
        .expect("on_activate produced a window state");

    css_parses_cleanly();
    startup_state(&st);
    highlight_rules(&st);
    metrics_rules(&st);
    menu_rules(&st);
    dimming_and_highlight(&st);
    selection_rules(&st);
    tab_rules(&st);
    layout_rules(&st);
    closing_rules(&st);
    about_rules(&st);
    config_rules(&st);

    st.force_close.set(true);
    st.window.close();
    pump(200);
    let _ = std::fs::remove_dir_all(&tmp);
}

fn css_parses_cleanly() {
    for (name, css) in [
        ("chrome", build_css("JetBrains Mono", DEFAULT_HIGHLIGHT, 5)),
        ("dark menu", CSS_DARK_MENU.to_string()),
    ] {
        let provider = CssProvider::new();
        let errors = Rc::new(RefCell::new(Vec::<String>::new()));
        {
            let errors = errors.clone();
            provider.connect_parsing_error(move |_, section, err| {
                errors.borrow_mut().push(format!("{section}: {err}"));
            });
        }
        provider.load_from_string(&css);
        assert!(errors.borrow().is_empty(), "{name} CSS has parse errors: {:?}", errors.borrow());
    }
}

/// The chrome stylesheet GTK is actually using.
fn live_css(st: &Rc<WindowState>) -> String {
    st.css_provider.to_str().to_string()
}

/// Whether `css` contains the colour `hex` (GTK re-serialises `#RRGGBB` as
/// `rgba(r,g,b,a)`, so match on the channel values).
fn css_has_color(css: &str, hex: &str) -> bool {
    let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    let compact: String = css.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains(&format!("rgba({},{},{}", c(0), c(2), c(4)))
        || compact.contains(&format!("rgb({},{},{}", c(0), c(2), c(4)))
        || compact.to_lowercase().contains(&format!("#{}", hex.to_lowercase()))
}

fn startup_state(st: &Rc<WindowState>) {
    assert_eq!(st.tabs.borrow().len(), 1, "starts with one tab");
    assert_eq!(st.notebook.n_pages(), 1);
    assert!(!st.notebook.shows_tabs(), "tab strip hidden with a single tab");
    let ps = panes(st);
    assert_eq!(ps.len(), 1, "starts with one pane (default layout single)");
    let p = &ps[0];
    assert!(p.wrap.has_css_class("focused"), "the only pane is focused");
    assert!(!p.toolbar.is_visible(), "no toolbar with one pane in one tab");
    assert!(!p.dim_overlay.is_visible(), "a lone pane is never dimmed");
    assert!(st.dim_non_selected.get(), "dimming defaults to on");
    assert_eq!(st.dim_percent.get(), 5, "dimming defaults to 5%");
    assert_eq!(*st.highlight_color.borrow(), "FF3399", "default highlight colour");
    assert!(st.highlight_enabled.get(), "highlight defaults to on");
    assert_eq!(st.highlight_size.get(), 2, "highlight border defaults to 2px");
    assert!(css_has_color(&live_css(st), "FF3399"), "highlight colour is in the live CSS: {}", live_css(st));
    assert!(st.window.is_visible());
}

fn metrics_rules(st: &Rc<WindowState>) {
    if !metrics::SUPPORTED {
        assert!(!st.metrics_enabled.get(), "metrics must stay off where unsupported");
        assert!(!st.metrics_label.is_visible());
        return;
    }
    // On by default, 5 s, shown straight in the title bar (no clicking).
    assert!(st.metrics_enabled.get(), "system metrics default to on");
    assert_eq!(st.metrics_interval.get(), 5, "default interval is 5 s");
    let label = st.metrics_label.clone();
    assert!(label.is_visible(), "metrics label shown at startup");
    let text = label.text().to_string();
    assert!(
        text.starts_with("load avg: [") && text.contains("cpu:") && text.contains("mem:") && text.contains("swap:"),
        "unexpected metrics text: {text:?}"
    );
    assert!(!text.contains("vram"), "VRAM is deliberately not shown");
    assert!(
        label.tooltip_text().is_some_and(|t| t.contains("Load average")),
        "tooltip carries the detail"
    );

    // Lives in the window's title bar, on the right (next to the window buttons).
    let titlebar = st.window.titlebar().expect("window has a title bar");
    assert!(titlebar.is::<gtk4::HeaderBar>(), "title bar is a header bar");
    assert!(label.is_ancestor(&titlebar), "metrics label is inside the title bar");
    let b = label.compute_bounds(&titlebar).expect("label bounds");
    let right_edge = b.x() + b.width();
    assert!(right_edge > titlebar.width() as f32 * 0.6, "metrics should hug the right end: {b:?}");
    assert!(b.x() + b.width() <= titlebar.width() as f32, "label overflows the title bar");

    title_rules(st);
    squeeze_rules(st);

    // The first reading is only a CPU baseline; a percentage appears promptly.
    pump(1400);
    assert!(
        !label.text().contains("cpu:--"),
        "CPU percentage never appeared: {:?}",
        label.text()
    );

    // Turning it off hides the label; on shows it again.
    st.metrics_enabled.set(false);
    restart_metrics(st);
    assert!(!label.is_visible(), "label hidden when disabled");
    st.metrics_enabled.set(true);
    restart_metrics(st);
    assert!(label.is_visible(), "label back when re-enabled");

    // Interval is honoured: with 1 s the label refreshes within ~1 s…
    st.metrics_interval.set(1);
    restart_metrics(st);
    pump(1300);
    label.set_text("stale");
    pump(1300);
    assert_ne!(label.text(), "stale", "1 s interval did not refresh");
    // …and with a long interval the old (1 s) timer is gone, so nothing refreshes.
    st.metrics_interval.set(30);
    restart_metrics(st);
    pump(1300); // the one-shot warm-up reading
    label.set_text("stale");
    pump(2200);
    assert_eq!(label.text(), "stale", "an old timer kept refreshing after the interval changed");
    st.metrics_interval.set(5);
    restart_metrics(st);
}

/// The focused-pane border thickness GTK actually computed for the pane's
/// wrapper (from the live stylesheet), in px.
fn pane_border(p: &Rc<Pane>) -> f32 {
    #[allow(deprecated)] // no non-deprecated way to read a computed border
    let b = p.wrap.style_context().border();
    assert_eq!(b.left(), b.right());
    assert_eq!(b.left(), b.top());
    assert_eq!(b.left(), b.bottom());
    b.left() as f32
}

fn find_named(root: &gtk4::Widget, name: &str) -> gtk4::Widget {
    find_widget(root, &|w| w.widget_name() == name)
        .unwrap_or_else(|| panic!("no widget named {name:?} in the Settings dialog"))
}

fn highlight_rules(st: &Rc<WindowState>) {
    let p = panes(st)[0].clone();
    pump(300);
    assert_eq!(pane_border(&p), 2.0, "focused pane border is 2px by default");

    // Size is live.
    st.highlight_size.set(9);
    apply_chrome_font(st);
    pump(300);
    assert_eq!(pane_border(&p), 9.0, "border size not applied");

    // Off: no border at all (and the pane gets that space back).
    st.highlight_enabled.set(false);
    apply_chrome_font(st);
    pump(300);
    assert_eq!(pane_border(&p), 0.0, "highlight off must remove the border");
    st.highlight_enabled.set(true);
    st.highlight_size.set(DEFAULT_BORDER_PX);
    apply_chrome_font(st);
    pump(300);
    assert_eq!(pane_border(&p), DEFAULT_BORDER_PX as f32);

    // Settings: the size + colour controls grey out with the switch, and the
    // switch / spin drive the state.
    open_settings(st);
    pump(500);
    let dialog = gtk4::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk4::Window>().ok())
        .find(|w| w.title().as_deref() == Some("skyterm Settings"))
        .expect("Settings dialog");
    let root: gtk4::Widget = dialog.clone().upcast();
    eprintln!("[settings] dialog {}x{}", dialog.width(), dialog.height());
    assert!(dialog.height() <= 560, "Settings dialog grew to {}px tall", dialog.height());
    let switch = find_named(&root, "setting-highlight-switch").downcast::<gtk4::Switch>().unwrap();
    let size = find_named(&root, "setting-highlight-size").downcast::<gtk4::SpinButton>().unwrap();
    let color = find_named(&root, "setting-highlight-color");
    let size_label = find_named(&root, "setting-highlight-size-label");
    let color_label = find_named(&root, "setting-highlight-color-label");
    let dependents = [size.upcast_ref::<gtk4::Widget>(), &color, &size_label, &color_label];

    assert!(switch.is_active(), "switch shows the current (on) state");
    assert_eq!(size.value(), 2.0, "spin shows the current size");
    assert!(dependents.iter().all(|w| w.is_sensitive()), "size/colour enabled while highlight is on");

    switch.set_active(false);
    pump(300);
    assert!(!st.highlight_enabled.get());
    assert!(dependents.iter().all(|w| !w.is_sensitive()), "size/colour must grey out when the highlight is off");
    assert_eq!(pane_border(&p), 0.0);

    switch.set_active(true);
    pump(300);
    assert!(st.highlight_enabled.get());
    assert!(dependents.iter().all(|w| w.is_sensitive()), "size/colour must re-enable with the highlight");
    assert_eq!(pane_border(&p), DEFAULT_BORDER_PX as f32);

    size.set_value(12.0);
    pump(300);
    assert_eq!(st.highlight_size.get(), 12);
    assert_eq!(pane_border(&p), 12.0);
    size.set_value(DEFAULT_BORDER_PX as f64);
    pump(300);

    // Opening Settings while the highlight is OFF starts with them greyed out.
    dialog.close();
    pump(300);
    st.highlight_enabled.set(false);
    open_settings(st);
    pump(500);
    let dialog = gtk4::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk4::Window>().ok())
        .find(|w| w.title().as_deref() == Some("skyterm Settings"))
        .expect("Settings dialog (2)");
    let root: gtk4::Widget = dialog.clone().upcast();
    assert!(!find_named(&root, "setting-highlight-size").is_sensitive());
    assert!(!find_named(&root, "setting-highlight-color").is_sensitive());
    assert!(!find_named(&root, "setting-highlight-switch").downcast::<gtk4::Switch>().unwrap().is_active());
    dialog.close();
    pump(300);
    st.highlight_enabled.set(true);
    apply_chrome_font(st);
    pump(200);
}

/// Height of the theme's stock title bar, in px. The bar must be at least 20%
/// shorter than that.
const STOCK_HEADER_HEIGHT: i32 = 46;

fn title_rules(st: &Rc<WindowState>) {
    let header = &st.header;
    eprintln!("[header] height {}", header.height());
    assert!(
        header.height() as f32 <= STOCK_HEADER_HEIGHT as f32 * 0.8,
        "title bar is {}px tall; it must be ≤ {:.0}px (20% shorter than the stock {STOCK_HEADER_HEIGHT}px)",
        header.height(),
        STOCK_HEADER_HEIGHT as f32 * 0.8
    );
    assert!(header.height() >= 30, "title bar too short to hold the window buttons: {}", header.height());
    // Everything still fits inside the shorter bar.
    let hb = header.height() as f32;
    let tb = st.title_label.compute_bounds(header).expect("title bounds");
    assert!(tb.y() >= 0.0 && tb.y() + tb.height() <= hb, "title is clipped by the bar: {tb:?} in {hb}");
    if st.metrics_label.is_visible() {
        let mb = st.metrics_label.compute_bounds(header).expect("metrics bounds");
        assert!(mb.y() >= 0.0 && mb.y() + mb.height() <= hb, "metrics clipped by the bar: {mb:?} in {hb}");
    }
    assert!(
        window_buttons_left(header) < header.width() as f32,
        "window buttons not found in the title bar"
    );
    let tb = st.title_label.compute_bounds(header).expect("title bounds");
    assert!(tb.x() < 60.0, "the skyterm title belongs on the left: {tb:?}");
    assert_eq!(st.title_label.text(), "skyterm");
    // Nothing in the centre any more.
    let centre = header.title_widget().expect("a (blank) centre widget");
    assert!(centre.first_child().is_none(), "the centred title should be empty");
    // Larger than the metrics text.
    let px = |l: &gtk4::Label| {
        let d = l.pango_context().font_description().expect("font");
        d.size() as f64 / gtk4::pango::SCALE as f64 * if d.is_size_absolute() { 1.0 } else { 96.0 / 72.0 }
    };
    assert!(px(&st.title_label) >= 15.0, "title font too small: {}", px(&st.title_label));
    assert!(px(&st.title_label) > px(&st.metrics_label), "title must be larger than the metrics");
}

/// Whenever the metrics are visible they must have ≥5px clear on both sides
/// and not be squeezed; otherwise they must be hidden entirely.
fn check_header_invariants(ctx: &str, st: &Rc<WindowState>) {
    let header = &st.header;
    let title = st.title_label.compute_bounds(header).expect("title bounds");
    assert!(!st.title_label.layout().is_ellipsized(), "{ctx}: title is squeezed");
    if !st.metrics_label.is_visible() {
        return;
    }
    let m = st.metrics_label.compute_bounds(header).expect("metrics bounds");
    eprintln!(
        "[{ctx}] window {} header {} title {:.0}..{:.0} metrics {:.0}..{:.0} buttons@{:.0} natural_text {}",
        st.window.width(), header.width(), title.x(), title.x() + title.width(),
        m.x(), m.x() + m.width(), window_buttons_left(header),
        st.metrics_label.create_pango_layout(Some(st.metrics_label.text().as_str())).pixel_size().0
    );
    let gap_left = m.x() - (title.x() + title.width());
    let gap_right = window_buttons_left(header) - (m.x() + m.width());
    assert!(gap_left >= 5.0, "{ctx}: metrics only {gap_left}px from the title");
    assert!(gap_right >= 5.0, "{ctx}: metrics only {gap_right}px from the window buttons");
    assert!(!st.metrics_label.layout().is_ellipsized(), "{ctx}: metrics text is squeezed");
}

fn squeeze_rules(st: &Rc<WindowState>) {
    st.window.set_default_size(1000, 400);
    pump(500);
    assert!(st.metrics_label.is_visible(), "metrics visible when there is plenty of room");
    check_header_invariants("1000px", st);

    // Squeeze step by step: at every width the metrics are either fully
    // readable with margins, or gone.
    let mut hidden_at = None;
    for w in [900, 800, 700, 600, 520, 460, 420, 380, 340, 300, 260, 200, 100] {
        st.window.set_default_size(w, 400);
        pump(500);
        check_header_invariants(&format!("{w}px"), st);
        if hidden_at.is_none() && !st.metrics_label.is_visible() {
            hidden_at = Some(w);
        }
    }
    assert!(hidden_at.is_some(), "metrics never hid, even at the minimum window width");
    assert!(!st.metrics_label.is_visible(), "metrics must be hidden at the minimum width");
    // The title and the window buttons stay readable at the minimum.
    check_header_invariants("minimum", st);
    assert!(
        st.title_label.compute_bounds(&st.header).unwrap().x() + st.title_label.width() as f32
            <= window_buttons_left(&st.header),
        "title overlaps the window buttons at the minimum width"
    );

    // Widening again brings them back (re-checked after the resize settles).
    for w in [300, 500, 700, 1000] {
        st.window.set_default_size(w, 400);
        pump(500);
        check_header_invariants(&format!("re-expand {w}px"), st);
    }
    assert!(st.metrics_label.is_visible(), "metrics did not return after re-expanding");
    st.window.set_default_size(1000, 640);
    pump(300);
}

fn menu_rules(st: &Rc<WindowState>) {
    let p = panes(st)[0].clone();

    // The very first popup, in a window too short for the menu to fit below
    // the pointer (the clipped-first-popup regression).
    st.window.set_default_size(700, 300);
    pump(500);
    let at = (80.0, 60.0);
    let pop = open_menu(st, &p, at);
    check_menu("lone pane, first popup, short window", st, &p, &pop, at);
    dismiss(&pop);
    st.window.set_default_size(1000, 640);
    pump(400);

    // Normal size, low enough that the menu can't fit below the pointer.
    let at = (300.0, 300.0);
    let pop = open_menu(st, &p, at);
    check_menu("lone pane", st, &p, &pop, at);
    dismiss(&pop);

    // Second popup, different spot.
    let at = (120.0, 40.0);
    let pop = open_menu(st, &p, at);
    check_menu("lone pane, second popup", st, &p, &pop, at);
    dismiss(&pop);

    // Split, then open the ORIGINAL pane's menu (the other regression: the
    // original pane's popover lost Close pane after a split). The new pane's
    // popover is shown for the first time in a short window.
    let new = split(st, &p, SplitDir::Right).expect("split right");
    pump(600);
    let at = (150.0, 250.0);
    let pop = open_menu(st, &p, at);
    check_menu("original pane after split", st, &p, &pop, at);
    dismiss(&pop);
    st.window.set_default_size(700, 300);
    pump(500);
    let at = (80.0, 60.0);
    let pop = open_menu(st, &new, at);
    check_menu("new pane, first popup, short window", st, &new, &pop, at);
    dismiss(&pop);
    st.window.set_default_size(1000, 640);
    pump(300);

    // Back to a single pane for the next sections.
    close_pane(st, &new);
    pump(300);
    assert_eq!(panes(st).len(), 1);
}

fn dimming_and_highlight(st: &Rc<WindowState>) {
    let a = panes(st)[0].clone();
    let b = split(st, &a, SplitDir::Down).expect("split down");
    pump(400);

    // The new pane holds focus; the old one is dimmed by the default 5%.
    assert!(b.wrap.has_css_class("focused") && !a.wrap.has_css_class("focused"));
    assert!(a.dim_overlay.is_visible(), "unfocused pane is dimmed");
    assert!((a.dim_overlay.opacity() - 0.05).abs() < 0.005, "dim is 5%, got {}", a.dim_overlay.opacity());
    assert!(!b.dim_overlay.is_visible(), "focused pane is not dimmed");
    assert!(a.toolbar.is_visible() && b.toolbar.is_visible(), "toolbars show with 2 panes");

    // Focus moves → dimming follows.
    focus_pane(st, &a);
    assert!(a.wrap.has_css_class("focused") && !b.wrap.has_css_class("focused"));
    assert!(!a.dim_overlay.is_visible() && b.dim_overlay.is_visible());

    // Percent setting applies live and clamps.
    st.dim_percent.set(20);
    apply_pane_dimming_all(st);
    assert!((b.dim_overlay.opacity() - 0.20).abs() < 0.005);
    st.dim_percent.set(99);
    apply_pane_dimming_all(st);
    assert!((b.dim_overlay.opacity() - MAX_DIM_PERCENT as f64 / 100.0).abs() < 0.005, "clamped");
    st.dim_percent.set(0);
    apply_pane_dimming_all(st);
    assert!(!b.dim_overlay.is_visible(), "0% = no scrim");

    // Switch off entirely.
    st.dim_percent.set(5);
    st.dim_non_selected.set(false);
    apply_pane_dimming_all(st);
    assert!(!a.dim_overlay.is_visible() && !b.dim_overlay.is_visible(), "dimming off");
    st.dim_non_selected.set(true);
    apply_pane_dimming_all(st);
    assert!(b.dim_overlay.is_visible());

    // The dim overlay must never swallow clicks meant for the terminal.
    assert!(!a.dim_overlay.can_target() && !a.dim_overlay.is_focusable());

    // Highlight colour changes the live stylesheet.
    *st.highlight_color.borrow_mut() = "00FF7F".to_string();
    apply_chrome_font(st);
    assert!(css_has_color(&live_css(st), "00FF7F"));
    assert!(!css_has_color(&live_css(st), "FF3399"));
    *st.highlight_color.borrow_mut() = DEFAULT_HIGHLIGHT.to_string();
    apply_chrome_font(st);

    close_pane(st, &b);
    pump(300);
    assert_eq!(panes(st).len(), 1);
    assert!(!panes(st)[0].dim_overlay.is_visible(), "survivor is undimmed");
    assert!(panes(st)[0].wrap.has_css_class("focused"), "survivor takes focus");
}

fn selection_rules(st: &Rc<WindowState>) {
    use crate::renderer::Selection;
    let a = panes(st)[0].clone();
    let b = split(st, &a, SplitDir::Right).expect("split");
    pump(300);

    // A selection belongs to the focused pane: moving focus clears the old
    // pane's selection so no stray highlighted block is left behind.
    focus_pane(st, &a);
    *a.selection.borrow_mut() = Some(Selection { anchor: (0, 0), active: (0, 3) });
    focus_pane(st, &b);
    assert!(a.selection.borrow().is_none(), "selection left behind in the unfocused pane");

    // Double-click on blank space selects nothing (no one-cell blocks).
    select_word(&b, 10, 10);
    assert!(b.selection.borrow().is_none(), "double-click on blank cell left a block");
    select_line(&b, 10);
    assert!(b.selection.borrow().is_none(), "triple-click on blank line left a block");

    close_pane(st, &b);
    pump(300);
}

fn tab_rules(st: &Rc<WindowState>) {
    assert_eq!(st.tabs.borrow().len(), 1);
    assert!(new_tab(st));
    pump(300);
    assert_eq!(st.tabs.borrow().len(), 2);
    assert_eq!(st.notebook.n_pages(), 2);
    assert!(st.notebook.shows_tabs(), "tab strip appears with 2 tabs");
    assert_eq!(st.notebook.current_page(), Some(1), "new tab is selected");
    assert!(
        panes(st)[0].toolbar.is_visible(),
        "a lone pane shows its toolbar once there are 2 tabs (drag handle between tabs)"
    );

    cycle_tab(st, 1);
    assert_eq!(st.notebook.current_page(), Some(0), "cycle wraps forward");
    cycle_tab(st, -1);
    assert_eq!(st.notebook.current_page(), Some(1), "cycle wraps back");

    // Tab limit.
    st.tab_max.set(2);
    st.tab_max_banner.set_visible(false);
    assert!(!new_tab(st), "tab limit refuses a third tab");
    assert_eq!(st.tabs.borrow().len(), 2);
    assert!(st.tab_max_banner.is_visible(), "limit banner shown");
    st.tab_max.set(20);

    // Closing a tab goes back to a single tab with the strip hidden.
    let second = st.tabs.borrow()[1].clone();
    close_tab(st, &second);
    pump(200);
    assert_eq!(st.tabs.borrow().len(), 1);
    assert!(!st.notebook.shows_tabs());
    assert!(!panes(st)[0].toolbar.is_visible(), "toolbar hides again with 1 pane in 1 tab");
}

fn layout_rules(st: &Rc<WindowState>) {
    for (layout, expect) in [(Layout::TwoH, 2), (Layout::TwoV, 2), (Layout::Three, 3), (Layout::Four, 4)] {
        assert!(new_tab(st));
        pump(200);
        let base = panes(st)[0].clone();
        apply_layout(st, &base, layout);
        pump(400);
        let ps = panes(st);
        assert_eq!(ps.len(), expect, "{layout:?} should build {expect} panes");
        // Exactly one focused, every other pane dimmed.
        assert_eq!(ps.iter().filter(|p| p.wrap.has_css_class("focused")).count(), 1);
        for p in &ps {
            assert_eq!(p.dim_overlay.is_visible(), !p.wrap.has_css_class("focused"));
        }
        let tab = cur_tab(st);
        close_tab(st, &tab);
        pump(200);
    }
    assert_eq!(st.tabs.borrow().len(), 1);
}

fn closing_rules(st: &Rc<WindowState>) {
    // With two panes, closing one leaves one, focused and undimmed.
    let a = panes(st)[0].clone();
    let b = split(st, &a, SplitDir::Right).expect("split");
    pump(300);
    assert_eq!(panes(st).len(), 2);
    close_pane(st, &b);
    pump(300);
    assert_eq!(panes(st).len(), 1);
    assert!(a.wrap.has_css_class("focused"));

    // Closing the only pane of the only tab must ask first (even with the
    // pane-close setting off) rather than quitting.
    st.confirm_pane_close.set(false);
    let before = toplevel_count();
    request_close_pane(st, &a);
    pump(600);
    assert!(toplevel_count() > before, "no confirmation dialog for the last pane");
    assert_eq!(st.tabs.borrow().len(), 1, "last pane closed without confirmation");
    assert!(st.window.is_visible(), "window closed without confirmation");
    // Dismiss the dialog (cancels).
    for w in gtk4::Window::list_toplevels() {
        if let Ok(win) = w.downcast::<gtk4::Window>() {
            if win.is::<gtk4::ApplicationWindow>() || win.has_css_class("skyterm-about") {
                continue;
            }
            win.close();
        }
    }
    pump(300);
    assert_eq!(panes(st).len(), 1, "cancelling kept the pane");

    // Two panes + setting off: closes immediately, no dialog.
    let b = split(st, &a, SplitDir::Right).expect("split");
    pump(300);
    let before = toplevel_count();
    request_close_pane(st, &b);
    pump(400);
    assert_eq!(toplevel_count(), before, "unexpected dialog with confirm_pane_close off");
    assert_eq!(panes(st).len(), 1);
    st.confirm_pane_close.set(true);
}

fn about_rules(st: &Rc<WindowState>) {
    let before: Vec<_> = gtk4::Window::list_toplevels();
    open_about(st);
    pump(600);
    let about = gtk4::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk4::Window>().ok())
        .find(|w| w.has_css_class("skyterm-about") && !before.iter().any(|b| b == w.upcast_ref::<gtk4::Widget>()))
        .expect("About window opened");
    assert!(!about.is_decorated(), "About draws its own rounded card");
    assert!(!about.is_modal(), "About is non-modal so clicking outside can dismiss it");
    assert!(!about.is_resizable());
    let root: gtk4::Widget = about.clone().upcast();
    let card = find_widget(&root, &|w| w.has_css_class("skyterm-about-card")).expect("rounded card");
    assert!(card.is_mapped());
    // Logo sits above the title.
    let logo = find_widget(&root, &|w| w.is::<gtk4::Image>() && w.width() >= 96).expect("logo image");
    let title = find_label(&root, "Skyterm").expect("title label");
    let lb = logo.compute_bounds(&root).unwrap();
    let tb = title.compute_bounds(&root).unwrap();
    assert!(lb.y() + lb.height() <= tb.y() + 1.0, "logo must be above the title");
    assert!(find_label(&root, &format!("Version {}", env!("CARGO_PKG_VERSION"))).is_some());
    assert!(find_widget(&root, &|w| w.is::<gtk4::LinkButton>()).is_some(), "GitHub link");
    // Close button + dismissal.
    let btn = find_widget(&root, &|w| w.has_css_class("skyterm-about-close")).expect("close button");
    btn.downcast::<gtk4::Button>().unwrap().emit_clicked();
    pump(300);
    assert!(!about.is_visible(), "About closes via its × button");
}

fn config_rules(st: &Rc<WindowState>) {
    st.dim_non_selected.set(false);
    st.dim_percent.set(12);
    *st.highlight_color.borrow_mut() = "12AB34".to_string();
    st.metrics_enabled.set(false);
    st.metrics_interval.set(9);
    st.highlight_enabled.set(false);
    st.highlight_size.set(7);
    save_config(st);
    let path = skyterm_core::config::Config::default_path().expect("config path");
    assert!(path.starts_with(std::env::var("XDG_CONFIG_HOME").unwrap()), "config must go to the temp dir");
    let cfg = skyterm_core::config::Config::load(&path);
    assert_eq!(cfg.dim_non_selected_panes, Some(false));
    assert_eq!(cfg.dim_non_selected_percent, Some(12));
    assert_eq!(cfg.selected_pane_highlight_color.as_deref(), Some("12AB34"));
    assert_eq!(cfg.system_metrics_enabled, Some(false));
    assert_eq!(cfg.system_metrics_interval_secs, Some(9));
    assert_eq!(cfg.selected_pane_highlight, Some(false));
    assert_eq!(cfg.selected_pane_highlight_border_size, Some(7));
    assert_eq!(cfg.font_path.as_deref().and_then(|p| p.to_str()), Some(font::EMBEDDED_FONT_PATH));
    st.dim_non_selected.set(true);
    st.dim_percent.set(5);
    *st.highlight_color.borrow_mut() = DEFAULT_HIGHLIGHT.to_string();
    st.metrics_enabled.set(metrics::SUPPORTED);
    st.metrics_interval.set(5);
    st.highlight_enabled.set(true);
    st.highlight_size.set(DEFAULT_BORDER_PX);
}
