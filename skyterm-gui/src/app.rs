use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Once;
use std::time::{Duration, Instant};

use gtk4::gio;
use gtk4::glib::object::Cast;
use gtk4::prelude::*;
use gtk4::{
    gdk, glib, Adjustment, Application, ApplicationWindow, CssProvider,
    Entry, EventControllerFocus, EventControllerKey, EventControllerScroll,
    EventControllerScrollFlags, GLArea,
    GestureClick, GestureDrag, Orientation, Paned, PopoverMenu, Scrollbar,
};
use skyterm_core::{
    theme::Theme,
    Grid, MouseMode, Parser,
};

use crate::font;
use crate::input;
use crate::pty;
use crate::renderer::{Renderer, Selection};

const INITIAL_COLS: u16 = 100;
const INITIAL_ROWS: u16 = 30;
const DEFAULT_FONT_SIZE_PX: u32 = 16;
const MIN_FONT_SIZE_PX: u32 = 6;
const MAX_FONT_SIZE_PX: u32 = 72;
const CHORD_TIMEOUT: Duration = Duration::from_secs(2);

/// Focus highlight on the wrapper around each pane (2px transparent border
/// when unfocused reserves the same space so panes don't visibly jump when
/// focus moves), plus a font-family override for the right-click menu and
/// tab-limit banner so GTK chrome renders in the same monospace face as the
/// terminal grid. `__FONT_FAMILY__` is substituted at runtime with the active
/// font's family name (see [`build_css`] / [`apply_chrome_font`]) and the
/// provider is reloaded on font changes.
const CSS_TEMPLATE: &str = "
.pane-wrap {
    border: 2px solid transparent;
}
.pane-wrap.focused {
    border-color: #5eb1ff;
}
popover.menu,
popover.menu label,
popover.menu modelbutton,
popover.menu modelbutton accelerator {
    font-family: __FONT_FAMILY__, monospace;
}
popover.menu modelbutton accelerator {
    color: rgba(255, 255, 255, 0.55);
    padding-left: 24px;
}
popover.menu scrollbar,
popover.menu scrollbar trough,
popover.menu scrollbar slider,
popover.menu overshoot,
popover.menu undershoot {
    opacity: 0;
    min-width: 0;
    min-height: 0;
    margin: 0;
    padding: 0;
}
button.pane-close-btn {
    color: #c0392b;
    background: none;
    border: none;
    box-shadow: none;
    padding: 6px 12px;
    margin: 0;
}
button.pane-close-btn:hover {
    background: rgba(192, 57, 43, 0.18);
}
.pane-toolbar {
    background: rgba(40, 40, 40, 0.85);
    border-radius: 14px;
    padding: 1px 4px;
    margin: 6px;
    opacity: 0.10;
}
.pane-toolbar.hovered {
    opacity: 1.0;
}
.pane-toolbar button {
    background: none;
    border: none;
    box-shadow: none;
    padding: 2px 6px;
    margin: 0;
    min-width: 16px;
    min-height: 18px;
    color: #dddddd;
}
.pane-toolbar button:hover {
    background: rgba(255, 255, 255, 0.16);
    border-radius: 8px;
}
.pane-toolbar button.pane-toolbar-close:hover {
    background: rgba(192, 57, 43, 0.55);
    color: #ffffff;
}
.pane-drop-highlight {
    background: rgba(46, 204, 113, 0.30);
    border: 2px solid rgba(46, 204, 113, 0.95);
    border-radius: 4px;
}
.tab-max-banner {
    background: rgba(255, 248, 180, 0.88);
    color: #4a3b00;
    border-radius: 6px;
    padding: 10px 16px;
    margin: 10px;
    font-family: __FONT_FAMILY__, monospace;
    font-size: 12px;
}
.tab-max-banner-close {
    color: #4a3b00;
    padding: 0;
    min-width: 20px;
    min-height: 20px;
}
.split-dir-btn {
    font-family: __FONT_FAMILY__, monospace;
    font-size: 14px;
    padding: 2px 8px;
    min-width: 30px;
    min-height: 26px;
}
";

#[derive(Clone, Copy)]
enum SplitDir {
    Left,
    Right,
    Up,
    Down,
}

struct WindowState {
    /// Every tab in the window. Each tab owns its own pane tree.
    tabs: RefCell<Vec<Rc<Tab>>>,
    /// `Some(t)` while a Ctrl+A prefix is waiting for its second key; cleared
    /// after the second key or after `CHORD_TIMEOUT`.
    chord_at: Cell<Option<Instant>>,
    /// The pane that owns the currently-open (or last-opened) context menu.
    /// Menu actions read this so they know which pane to operate on.
    menu_target: RefCell<Option<Rc<Pane>>>,
    /// Right-click menu when there are multiple panes (includes "Close pane").
    split_menu: gio::Menu,
    /// Same menu without the close section — used when this would close the
    /// last pane in the last tab (i.e. would empty the window).
    split_menu_no_close: gio::Menu,
    /// Built-in presets + any user themes discovered in the themes folder.
    /// Settings reads from this list; first entry is the new-install default.
    available_themes: Vec<Theme>,
    /// Names of themes that came from the user themes folder (as opposed to
    /// the built-in `Theme::presets()`). Used by menu / Settings to bucket
    /// these under "Custom" instead of Dark / Light.
    user_theme_names: std::collections::HashSet<String>,
    /// The toplevel window. Popovers parent here for sizing freedom, not to
    /// the per-pane `GLArea`.
    window: ApplicationWindow,
    /// Tab container. Each notebook page hosts one `Tab`'s pane tree.
    notebook: gtk4::Notebook,
    /// Monotonic counter for tab labels. Starts at 1, only goes up — closing
    /// a tab does not free its number, so labels are stable for the lifetime
    /// of each tab and never collide.
    next_tab_number: Cell<u32>,
    /// The active color theme. Shared by-Rc with every pane so swapping the
    /// theme retints all existing cells (because cells store `CellColor`
    /// palette indices, not raw RGB).
    theme: Rc<RefCell<Theme>>,
    /// Current font path. Mutable so Settings can swap font families live.
    font_path: Rc<RefCell<PathBuf>>,
    /// Whether cursor blinking is enabled.
    cursor_blink: Rc<Cell<bool>>,
    /// Current blink phase: true = cursor visible. Flipped every 500 ms by the
    /// blink timer. Always true when blinking is disabled.
    blink_phase: Rc<Cell<bool>>,
    /// Whether double-click-word / triple-click-line selection is enabled.
    click_word_select: Rc<Cell<bool>>,
    /// Whether making a selection copies it to the clipboard automatically.
    copy_on_select: Rc<Cell<bool>>,
    /// Maximum number of tabs. Default 20; overridable via `tab_max_number` in config.
    tab_max: Cell<u32>,
    /// Banner shown in the top-right when the tab limit is reached.
    tab_max_banner: gtk4::Box,
    /// Active hide-timer for the tab-max banner. Cancelled and replaced on each show.
    tab_max_banner_timer: RefCell<Option<glib::SourceId>>,
    /// Show a confirmation dialog before closing a tab.
    confirm_tab_close: Cell<bool>,
    /// Show a confirmation dialog before closing a pane.
    confirm_pane_close: Cell<bool>,
    /// Show a confirmation dialog before closing the window via the OS button.
    confirm_window_close: Cell<bool>,
    /// Set to `true` just before programmatically closing the window so the
    /// `close-request` handler lets it through without showing a second dialog.
    force_close: Cell<bool>,
    /// The pane that is currently being dragged via the toolbar handle, or
    /// `None`. Set in `DragSource::prepare`, cleared in `drag-end`. Drop
    /// targets read this to know which pane to rearrange and to reject
    /// drops on the source pane itself.
    dragging: RefCell<Option<Rc<Pane>>>,
    /// CSS provider for the chrome stylesheet. Held so the font-family of
    /// menus / banners can be re-substituted when the user picks a different
    /// terminal font in Settings.
    css_provider: CssProvider,
}

struct Tab {
    /// The notebook page widget — holds the pane tree root.
    container: gtk4::Box,
    panes: RefCell<Vec<Rc<Pane>>>,
    /// Which pane was last focused inside this tab. Restored when switching
    /// back to the tab.
    focused: RefCell<Option<Rc<Pane>>>,
    /// The widget shown in the notebook's tab strip (label + close button).
    tab_label: gtk4::Box,
}

struct Pane {
    /// CSS-styled wrapper that participates in the `Paned` tree. The focused
    /// pane has the `focused` class added to this widget.
    wrap: gtk4::Box,
    gl_area: GLArea,
    /// Floating toolbar over the top-right of the gl_area. Hidden when this
    /// pane is the only one in its tab; shown otherwise. Holds the drag and
    /// close icons.
    toolbar: gtk4::Box,
    /// Translucent green overlay shown during pane drag-and-drop to indicate
    /// which edge of this pane the drop would land on. Half the pane's size,
    /// pinned to the corresponding edge via halign/valign. Pointer-transparent
    /// so it doesn't block the underlying drop-target motion events.
    drop_highlight: gtk4::Box,
    scrollbar: Scrollbar,
    scroll_adj: Adjustment,
    scroll_syncing: Cell<bool>,
    grid: RefCell<Grid>,
    parser: RefCell<Parser>,
    writer: RefCell<Box<dyn Write + Send>>,
    master: RefCell<Box<dyn portable_pty::MasterPty + Send>>,
    renderer: RefCell<Option<Renderer>>,
    font_size: Cell<u32>,
    cell_dims: Cell<(u32, u32)>,
    /// Shared with [`WindowState::font_path`] so changing font family from
    /// Settings updates every pane on next rebuild.
    font_path: Rc<RefCell<PathBuf>>,
    /// Shared with [`WindowState::theme`] so theme swaps retint live.
    theme: Rc<RefCell<Theme>>,
    /// Drag-to-select range in view-relative cell coordinates. Cleared on
    /// typing or scrolling so it doesn't drift out of sync with the view.
    selection: RefCell<Option<Selection>>,
    /// True when this pane holds keyboard focus. Only the focused pane blinks
    /// its cursor; unfocused panes show a static (non-blinking) cursor.
    is_focused: Cell<bool>,
    /// Pending debounced reflow timer (see `schedule_reflow`). Holds the latest
    /// timeout so a burst of resize events collapses into one grid+PTY resize.
    resize_source: RefCell<Option<glib::SourceId>>,
    /// Multi-click tracking for word/line selection: (last press time in ms,
    /// row, col, consecutive count). Used to distinguish single/double/triple
    /// clicks on the left button without a separate `GestureClick`.
    click_state: Cell<(u32, usize, usize, u8)>,
    _child: RefCell<Box<dyn portable_pty::Child + Send + Sync>>,
}

pub fn on_activate(app: &Application) {
    // Install our app icon into a user-writable icon theme directory so the
    // window manager / shell can pick it up. Best-effort; ignore failure.
    install_app_icon();

    // Load persisted settings before doing anything else, so the first pane
    // uses the saved font / theme / scrollback rather than defaults.
    let cfg = skyterm_core::config::Config::default_path()
        .map(|p| skyterm_core::config::Config::load(&p))
        .unwrap_or_default();

    let font_path = match cfg.font_path.clone() {
        Some(p) if p.is_file() => p,
        _ => font::locate_monospace_font().unwrap_or_else(|_| {
            std::path::PathBuf::from(font::EMBEDDED_FONT_PATH)
        }),
    };
    if font_path.to_str() == Some(font::EMBEDDED_FONT_PATH) {
        log::info!("skyterm starting — font: embedded JetBrains Mono Regular");
    } else {
        log::info!("skyterm starting — font {}", font_path.display());
    }

    let initial_family =
        font::family_name(&font_path).unwrap_or_else(|_| "monospace".to_string());
    let css_provider = install_css(&initial_family);

    // First-pane atlas just so we can size the initial window sensibly.
    let initial_atlas = match font::build_atlas(&font_path, DEFAULT_FONT_SIZE_PX) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("skyterm: failed to build font atlas: {e}");
            return;
        }
    };

    let window = ApplicationWindow::builder()
        .application(app)
        .title("skyterm")
        .icon_name("skyterm")
        .default_width((INITIAL_COLS as u32 * initial_atlas.cell_w) as i32)
        .default_height((INITIAL_ROWS as u32 * initial_atlas.cell_h) as i32)
        .build();

    // Themes are needed both for the submenu and for WindowState. Built-in
    // presets first, then any user themes from `~/.config/skyterm/themes/*.toml`.
    // The names of user themes are tracked separately so the menu and the
    // Settings dialog can put them in a "Custom" section.
    let mut available_themes: Vec<Theme> = Theme::presets();
    let user_themes = load_user_themes();
    let user_theme_names: std::collections::HashSet<String> =
        user_themes.iter().map(|t| t.name.clone()).collect();
    available_themes.extend(user_themes);

    // Build the two menu variants. They share the splits + clipboard
    // sections (gio::Menu sections are referenced, not copied), so we only
    // need to define them once.
    // Terminator-style split naming: "Horizontally" = horizontal divider line
    // = new pane below; "Vertically" = vertical divider line = new pane to the
    // right. The four directional actions (split-left/right/up/down) used by
    // the chord keybindings are still registered separately in
    // `install_pane_actions` but no longer surfaced in the menu.
    let splits = gio::Menu::new();
    splits.append(Some("Split Horizontally"), Some("pane.split-horizontal"));
    splits.append(Some("Split Vertically"),   Some("pane.split-vertical"));
    splits.append(Some("New Tab"),            Some("pane.new-tab"));
    splits.append(Some("New Window"),         Some("pane.new-window"));

    let clipboard = gio::Menu::new();
    clipboard.append(Some("Copy"), Some("pane.copy"));
    clipboard.append(Some("Paste"), Some("pane.paste"));
    clipboard.append(Some("Select All"), Some("pane.select-all"));

    // Themes submenu — Dark / Light for built-in presets, Custom (flat) for
    // user-loaded themes regardless of brightness. Custom is omitted when no
    // user themes were found.
    let dark_items = gio::Menu::new();
    let light_items = gio::Menu::new();
    let custom_items = gio::Menu::new();
    for t in &available_themes {
        let item = gio::MenuItem::new(Some(t.name.as_str()), None);
        item.set_action_and_target_value(
            Some("pane.set-theme"),
            Some(&t.name.to_variant()),
        );
        if user_theme_names.contains(&t.name) {
            custom_items.append_item(&item);
        } else if t.is_dark() {
            dark_items.append_item(&item);
        } else {
            light_items.append_item(&item);
        }
    }
    let themes_submenu = gio::Menu::new();
    themes_submenu.append_section(Some("Dark"), &dark_items);
    themes_submenu.append_section(Some("Light"), &light_items);
    if !user_theme_names.is_empty() {
        themes_submenu.append_section(Some("Custom"), &custom_items);
    }

    let prefs = gio::Menu::new();
    prefs.append_submenu(Some("Themes"), &themes_submenu);
    prefs.append(Some("Settings…"), Some("pane.settings"));
    prefs.append(Some("About…"), Some("pane.about"));

    // The destructive-action section uses a custom widget so we can render
    // the red close styling the standard menu model can't express.
    let danger = gio::Menu::new();
    let close_item = gio::MenuItem::new(None, None);
    close_item.set_attribute_value("custom", Some(&"close-pane".to_variant()));
    danger.append_item(&close_item);

    let split_menu = gio::Menu::new();
    split_menu.append_section(None, &splits);
    split_menu.append_section(None, &clipboard);
    split_menu.append_section(None, &prefs);
    split_menu.append_section(None, &danger);

    let split_menu_no_close = gio::Menu::new();
    split_menu_no_close.append_section(None, &splits);
    split_menu_no_close.append_section(None, &clipboard);
    split_menu_no_close.append_section(None, &prefs);

    // Tab container. The window's child is always the notebook; per-tab
    // pane trees live inside notebook pages.
    let notebook = gtk4::Notebook::new();
    notebook.set_hexpand(true);
    notebook.set_vexpand(true);
    notebook.set_scrollable(true);
    notebook.set_show_tabs(false);
    notebook.set_show_border(false);

    // Overlay wraps the notebook so we can float the tab-max banner on top.
    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&notebook));

    let banner_text = gtk4::Label::new(Some(
        "Maximum number of tabs has been opened.\n\
         To change this, edit ~/.config/skyterm/config.toml  (tab_max_number)",
    ));
    banner_text.set_hexpand(true);
    banner_text.set_xalign(0.0);

    let banner_close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    banner_close_btn.set_has_frame(false);
    banner_close_btn.add_css_class("flat");
    banner_close_btn.add_css_class("tab-max-banner-close");
    banner_close_btn.set_valign(gtk4::Align::Start);

    let tab_max_banner = gtk4::Box::new(Orientation::Horizontal, 12);
    tab_max_banner.append(&banner_text);
    tab_max_banner.append(&banner_close_btn);
    tab_max_banner.set_halign(gtk4::Align::End);
    tab_max_banner.set_valign(gtk4::Align::Start);
    tab_max_banner.add_css_class("tab-max-banner");
    tab_max_banner.set_visible(false);
    overlay.add_overlay(&tab_max_banner);

    window.set_child(Some(&overlay));

    let initial_theme = cfg
        .theme_name
        .as_deref()
        .and_then(|name| available_themes.iter().find(|t| t.name == name).cloned())
        .unwrap_or_else(Theme::default);

    let cursor_blink = Rc::new(Cell::new(cfg.cursor_blink.unwrap_or(true)));
    let blink_phase = Rc::new(Cell::new(true));
    let click_word_select = Rc::new(Cell::new(cfg.click_word_select.unwrap_or(true)));
    let copy_on_select = Rc::new(Cell::new(cfg.copy_on_select.unwrap_or(false)));

    let tab_max = cfg.tab_max_number.unwrap_or(20);
    let confirm_tab_close = cfg.confirm_tab_close.unwrap_or(true);
    let confirm_pane_close = cfg.confirm_pane_close.unwrap_or(true);
    let confirm_window_close = cfg.confirm_window_close.unwrap_or(true);

    let state = Rc::new(WindowState {
        tabs: RefCell::new(Vec::new()),
        chord_at: Cell::new(None),
        menu_target: RefCell::new(None),
        split_menu,
        split_menu_no_close,
        window: window.clone(),
        notebook: notebook.clone(),
        next_tab_number: Cell::new(1),
        theme: Rc::new(RefCell::new(initial_theme)),
        font_path: Rc::new(RefCell::new(font_path)),
        available_themes,
        user_theme_names,
        cursor_blink: cursor_blink.clone(),
        blink_phase: blink_phase.clone(),
        click_word_select: click_word_select.clone(),
        copy_on_select: copy_on_select.clone(),
        tab_max: Cell::new(tab_max),
        tab_max_banner,
        tab_max_banner_timer: RefCell::new(None),
        confirm_tab_close: Cell::new(confirm_tab_close),
        confirm_pane_close: Cell::new(confirm_pane_close),
        confirm_window_close: Cell::new(confirm_window_close),
        force_close: Cell::new(false),
        dragging: RefCell::new(None),
        css_provider,
    });

    // Dismiss button on the tab-max banner.
    {
        let state_w = Rc::downgrade(&state);
        banner_close_btn.connect_clicked(move |_| {
            if let Some(s) = state_w.upgrade() {
                s.tab_max_banner.set_visible(false);
                if let Some(id) = s.tab_max_banner_timer.borrow_mut().take() {
                    id.remove();
                }
            }
        });
    }

    // Window close-request (OS title-bar X button). If confirm_window_close is
    // on, intercept and show a dialog; only close for real when confirmed.
    // force_close bypasses the dialog when the window is being closed
    // programmatically after the user already confirmed a tab/pane close.
    {
        let state = state.clone();
        window.connect_close_request(move |_win| {
            if state.force_close.get() {
                state.force_close.set(false);
                return glib::Propagation::Proceed;
            }
            if state.confirm_window_close.get() {
                let state = state.clone();
                confirm_close(
                    &state.window.clone(),
                    "Are you sure you want to close skyterm?",
                    move || {
                        state.force_close.set(true);
                        state.window.close();
                    },
                );
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    // Cursor blink tick (500 ms). Only does work when blinking is enabled:
    // flips the phase and re-renders the current tab's panes.
    {
        let state_w = Rc::downgrade(&state);
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(state) = state_w.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.cursor_blink.get() {
                state.blink_phase.set(!state.blink_phase.get());
                if let Some(tab) = current_tab(&state) {
                    for p in tab.panes.borrow().iter() {
                        p.gl_area.queue_render();
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    install_pane_actions(&window, &state);
    install_accelerators(app);

    // Tab switch → restore keyboard focus to that tab's last-focused pane.
    {
        let state = state.clone();
        notebook.connect_switch_page(move |_, page, _| {
            let tab = state
                .tabs
                .borrow()
                .iter()
                .find(|t| t.container.upcast_ref::<gtk4::Widget>() == page)
                .cloned();
            if let Some(t) = tab {
                if let Some(p) = t.focused.borrow().clone() {
                    p.gl_area.grab_focus();
                }
            }
        });
    }

    // Tab drag reorder → keep state.tabs in sync with the notebook page order.
    {
        let state = state.clone();
        notebook.connect_page_reordered(move |_, child, new_pos| {
            let new_pos = new_pos as usize;
            let mut tabs = state.tabs.borrow_mut();
            if let Some(old_pos) = tabs
                .iter()
                .position(|t| t.container.upcast_ref::<gtk4::Widget>() == child)
            {
                let tab = tabs.remove(old_pos);
                tabs.insert(new_pos, tab);
            }
        });
    }

    if !new_tab(&state) {
        return;
    }

    // Apply persisted font size / scrollback after the first pane exists,
    // since both operate on per-pane state.
    if let Some(size) = cfg.font_size {
        apply_font_size_all(&state, size);
    }
    if let Some(lines) = cfg.scrollback_lines {
        apply_scrollback_all(&state, lines);
    }

    window.present();
}

/// The pane an action should operate on. When a popover is open, that's the
/// pane it was opened from (`menu_target`); when an action is fired by a
/// keyboard accelerator with no popover, fall back to the currently focused
/// pane. Used so registered accelerators work the same way the menu does.
fn action_target(state: &Rc<WindowState>) -> Option<Rc<Pane>> {
    state
        .menu_target
        .borrow()
        .clone()
        .or_else(|| focused_pane(state))
}

/// Register the `pane.*` actions on the window. Each one resolves a target
/// pane via [`action_target`] — popover-target when the menu was used,
/// focused pane otherwise — so the same actions cover both the right-click
/// menu and the application-level accelerators.
fn install_pane_actions(window: &ApplicationWindow, state: &Rc<WindowState>) {
    let group = gio::SimpleActionGroup::new();

    // Two named splits matching the menu items. "Horizontal" = horizontal
    // divider = new pane below (SplitDir::Down); "Vertical" = vertical
    // divider = new pane to the right (SplitDir::Right). Mirrors the
    // Terminator convention the user is migrating from.
    for (name, dir) in [
        ("split-horizontal", SplitDir::Down),
        ("split-vertical", SplitDir::Right),
    ] {
        let action = gio::SimpleAction::new(name, None);
        let state = state.clone();
        action.connect_activate(move |_, _| {
            if let Some(target) = action_target(&state) {
                split(&state, &target, dir);
            }
        });
        group.add_action(&action);
    }

    let close = gio::SimpleAction::new("close", None);
    {
        let state = state.clone();
        close.connect_activate(move |_, _| {
            if let Some(target) = action_target(&state) {
                request_close_pane(&state, &target);
            }
        });
    }
    group.add_action(&close);

    let copy = gio::SimpleAction::new("copy", None);
    {
        let state = state.clone();
        copy.connect_activate(move |_, _| {
            if let Some(target) = action_target(&state) {
                copy_selection(&target);
            }
        });
    }
    group.add_action(&copy);

    let paste_action = gio::SimpleAction::new("paste", None);
    {
        let state = state.clone();
        paste_action.connect_activate(move |_, _| {
            if let Some(target) = action_target(&state) {
                paste(&target, false);
            }
        });
    }
    group.add_action(&paste_action);

    let select_all = gio::SimpleAction::new("select-all", None);
    {
        let state = state.clone();
        select_all.connect_activate(move |_, _| {
            if let Some(target) = action_target(&state) {
                select_all_pane(&target);
            }
        });
    }
    group.add_action(&select_all);

    let new_tab_action = gio::SimpleAction::new("new-tab", None);
    {
        let state = state.clone();
        new_tab_action.connect_activate(move |_, _| {
            new_tab(&state);
        });
    }
    group.add_action(&new_tab_action);

    // Spin up another top-level skyterm window inside the same `Application`.
    // Re-entering `on_activate` reuses the activation path the very first
    // window took, so a new-window window is identical to a freshly-launched
    // one (fresh config load, fresh tab, its own pane tree).
    let new_window_action = gio::SimpleAction::new("new-window", None);
    {
        let state = state.clone();
        new_window_action.connect_activate(move |_, _| {
            if let Some(app) = state.window.application() {
                on_activate(&app);
            }
        });
    }
    group.add_action(&new_window_action);

    let settings_action = gio::SimpleAction::new("settings", None);
    {
        let state = state.clone();
        settings_action.connect_activate(move |_, _| {
            open_settings(&state);
        });
    }
    group.add_action(&settings_action);

    let about_action = gio::SimpleAction::new("about", None);
    {
        let state = state.clone();
        about_action.connect_activate(move |_, _| {
            open_about(&state);
        });
    }
    group.add_action(&about_action);

    // Applies a named theme to the right-clicked pane only (per-pane theme).
    let set_theme = gio::SimpleAction::new("set-theme", Some(glib::VariantTy::STRING));
    {
        let state = state.clone();
        set_theme.connect_activate(move |_, param| {
            let name = param
                .and_then(|v| v.get::<String>())
                .unwrap_or_default();
            let Some(pane) = state.menu_target.borrow().clone() else { return };
            if let Some(theme) = state
                .available_themes
                .iter()
                .find(|t| t.name == name)
                .cloned()
            {
                // Colors remain per-pane (the right-click set-theme is a
                // per-pane override). Font / size, however, are global state
                // — if the theme bundles them, apply globally so the user
                // actually sees the font swap they asked for.
                *pane.theme.borrow_mut() = theme.clone();
                pane.gl_area.queue_render();
                apply_theme_font(&state, &theme);
            }
        });
    }
    group.add_action(&set_theme);

    window.insert_action_group("pane", Some(&group));
}

/// Register application-level keyboard accelerators for the menu actions.
/// GTK4 auto-renders the accel string next to the corresponding menu items
/// (in a dimmed colour, Terminator-style), and routes the keypress to the
/// `pane.*` action group on the window. Uses `<Primary>` so the modifier
/// resolves to Ctrl on Linux/Windows and ⌘ on macOS.
fn install_accelerators(app: &Application) {
    let bindings: &[(&str, &[&str])] = &[
        // Splits + new tab — Terminator-compatible chords.
        ("pane.split-horizontal", &["<Primary><Shift>o"]),
        ("pane.split-vertical",   &["<Primary><Shift>e"]),
        ("pane.new-tab",          &["<Primary><Shift>t"]),
        // Clipboard. The same chords are also handled directly in
        // `wire_pane`'s key controller; the app-level binding takes
        // precedence and dispatches through `action_target` instead, which
        // routes to the focused pane just like the menu path does.
        ("pane.copy",             &["<Primary><Shift>c"]),
        ("pane.paste",            &["<Primary><Shift>v"]),
        ("pane.select-all",       &["<Primary><Shift>a"]),
        ("pane.close",            &["<Primary><Shift>w"]),
    ];
    for (action, keys) in bindings {
        app.set_accels_for_action(action, keys);
    }
}

/// The currently-active tab, or `None` if the notebook has no pages (window
/// is in the process of closing).
fn current_tab(state: &Rc<WindowState>) -> Option<Rc<Tab>> {
    let idx = state.notebook.current_page()? as usize;
    state.tabs.borrow().get(idx).cloned()
}

fn tab_of_pane(state: &Rc<WindowState>, pane: &Rc<Pane>) -> Option<Rc<Tab>> {
    state
        .tabs
        .borrow()
        .iter()
        .find(|t| t.panes.borrow().iter().any(|p| Rc::ptr_eq(p, pane)))
        .cloned()
}

/// The pane that should receive keyboard input window-wide. Equivalent to
/// the active tab's last-focused pane.
fn focused_pane(state: &Rc<WindowState>) -> Option<Rc<Pane>> {
    current_tab(state).and_then(|t| t.focused.borrow().clone())
}

/// Cycle keyboard focus to the next pane within the active tab. Wraps at
/// the end. No-op if there's only one pane.
fn focus_next_pane(state: &Rc<WindowState>) {
    let Some(tab) = current_tab(state) else {
        return;
    };
    let panes = tab.panes.borrow().clone();
    if panes.len() <= 1 {
        return;
    }
    let cur_idx = tab
        .focused
        .borrow()
        .as_ref()
        .and_then(|f| panes.iter().position(|p| Rc::ptr_eq(p, f)))
        .unwrap_or(0);
    let next = panes[(cur_idx + 1) % panes.len()].clone();
    focus_pane(state, &next);
}

/// Move focus to the pane spatially nearest in `dir` relative to the
/// currently-focused pane (by widget allocation, not by tree adjacency).
/// Picks the closest pane whose center lies in the requested half-plane,
/// with a small bonus for cross-axis alignment so adjacent rows/columns are
/// preferred over diagonal matches.
fn focus_direction(state: &Rc<WindowState>, dir: SplitDir) {
    let Some(tab) = current_tab(state) else {
        return;
    };
    let Some(focused) = tab.focused.borrow().clone() else {
        return;
    };
    let Some(f_bounds) = focused.gl_area.compute_bounds(&tab.container) else {
        return;
    };
    let fx = f_bounds.x() + f_bounds.width() / 2.0;
    let fy = f_bounds.y() + f_bounds.height() / 2.0;

    let panes = tab.panes.borrow().clone();
    let mut best: Option<(Rc<Pane>, f32)> = None;
    for p in panes.iter() {
        if Rc::ptr_eq(p, &focused) {
            continue;
        }
        let Some(b) = p.gl_area.compute_bounds(&tab.container) else {
            continue;
        };
        let cx = b.x() + b.width() / 2.0;
        let cy = b.y() + b.height() / 2.0;
        let dx = cx - fx;
        let dy = cy - fy;
        let (in_dir, primary, secondary) = match dir {
            SplitDir::Left => (dx < 0.0, -dx, dy.abs()),
            SplitDir::Right => (dx > 0.0, dx, dy.abs()),
            SplitDir::Up => (dy < 0.0, -dy, dx.abs()),
            SplitDir::Down => (dy > 0.0, dy, dx.abs()),
        };
        if !in_dir {
            continue;
        }
        let score = primary + secondary * 2.0;
        if best.as_ref().map(|(_, s)| score < *s).unwrap_or(true) {
            best = Some((p.clone(), score));
        }
    }
    if let Some((p, _)) = best {
        focus_pane(state, &p);
    }
}

/// Show a modal confirmation dialog with "Cancel" and "Close" buttons. Calls
/// `on_confirm` only if the user clicks Close.
fn confirm_close(
    window: &ApplicationWindow,
    message: &str,
    on_confirm: impl FnOnce() + 'static,
) {
    let dialog = gtk4::AlertDialog::builder()
        .message(message)
        .build();
    dialog.set_buttons(&["Cancel", "Close"]);
    dialog.set_cancel_button(0);
    dialog.set_default_button(0);
    dialog.choose(Some(window), None::<&gio::Cancellable>, move |result| {
        if result == Ok(1) {
            on_confirm();
        }
    });
}

/// Show the tab-limit banner in the top-right corner and hide it after 15 s.
/// Calling this while the banner is already visible resets the 15-second timer.
fn show_tab_max_banner(state: &Rc<WindowState>) {
    state.tab_max_banner.set_visible(true);
    if let Some(id) = state.tab_max_banner_timer.borrow_mut().take() {
        id.remove();
    }
    let state_w = Rc::downgrade(state);
    let id = glib::timeout_add_local_once(Duration::from_secs(15), move || {
        if let Some(s) = state_w.upgrade() {
            s.tab_max_banner.set_visible(false);
            *s.tab_max_banner_timer.borrow_mut() = None;
        }
    });
    *state.tab_max_banner_timer.borrow_mut() = Some(id);
}

/// Spawn a new tab with one fresh pane and make it active. Returns false if
/// the tab limit is reached or PTY / atlas / GLArea setup failed.
fn new_tab(state: &Rc<WindowState>) -> bool {
    if state.tabs.borrow().len() >= state.tab_max.get() as usize {
        show_tab_max_banner(state);
        return false;
    }
    let tab = match make_tab(state.clone(), INITIAL_COLS, INITIAL_ROWS) {
        Some(t) => t,
        None => return false,
    };
    let page = state
        .notebook
        .append_page(&tab.container, Some(&tab.tab_label));
    state.notebook.set_tab_reorderable(&tab.container, true);
    state.tabs.borrow_mut().push(tab.clone());
    state.notebook.set_show_tabs(state.tabs.borrow().len() > 1);
    state.notebook.set_current_page(Some(page));
    if let Some(p) = tab.panes.borrow().first().cloned() {
        focus_pane(state, &p);
    }
    // Tab count changed — sole-pane tabs may now need to show their toolbar.
    update_all_pane_toolbars(state);
    true
}

/// Build a tab: one initial pane, a tab-strip label widget with a close
/// button. Panes are wired here so callers don't need to.
fn make_tab(state: Rc<WindowState>, cols: u16, rows: u16) -> Option<Rc<Tab>> {
    let pane = make_pane(state.clone(), cols, rows)?;

    let container = gtk4::Box::new(Orientation::Horizontal, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.append(&pane.wrap);

    let tab_number = state.next_tab_number.get();
    state.next_tab_number.set(tab_number + 1);
    let title_label = gtk4::Label::new(Some(&format!("Tab {tab_number}")));
    title_label.set_xalign(0.0);

    // Pre-create the rename entry as a hidden sibling in the same box.
    // Toggling visibility (never add/remove) avoids the re-entrant GTK signals
    // that fire during gtk_box_remove and cause Gtk-CRITICAL parent-lookup failures.
    let title_entry = Entry::new();
    title_entry.set_max_width_chars(32);
    title_entry.set_visible(false);

    let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    close_btn.set_has_frame(false);
    close_btn.add_css_class("flat");

    let tab_label = gtk4::Box::new(Orientation::Horizontal, 6);
    tab_label.set_size_request(220, -1);
    title_label.set_hexpand(true);
    tab_label.append(&title_label);
    tab_label.append(&title_entry);
    tab_label.append(&close_btn);

    // Rename interaction — double-click label to start, Enter/focus-out to
    // commit, Escape to cancel. The entry's own visibility is the guard:
    // if it is already hidden when a handler fires, the rename is already done.
    {
        let lbl_w = title_label.clone();
        let ent_w = title_entry.clone();
        let gl_area_w = pane.gl_area.downgrade();

        // commit: save, swap back to label, restore terminal focus
        let commit: Rc<dyn Fn()> = Rc::new({
            let lbl = lbl_w.clone();
            let ent = ent_w.clone();
            let gla = gl_area_w.clone();
            move || {
                if !gtk4::prelude::WidgetExt::is_visible(&ent) { return; }
                let text = ent.text().trim().to_string();
                if !text.is_empty() { lbl.set_text(&text); }
                ent.set_visible(false);
                lbl.set_visible(true);
                if let Some(gl) = gla.upgrade() { gl.grab_focus(); }
            }
        });

        // Enter → commit
        {
            let c = commit.clone();
            title_entry.connect_activate(move |_| c());
        }

        // Escape → cancel (revert, don't save)
        let key_ctrl = EventControllerKey::new();
        key_ctrl.connect_key_pressed({
            let lbl = lbl_w.clone();
            let ent = ent_w.clone();
            let gla = gl_area_w.clone();
            move |_, keyval, _, _| {
                if keyval == gdk::Key::Escape {
                    if gtk4::prelude::WidgetExt::is_visible(&ent) {
                        ent.set_visible(false);
                        lbl.set_visible(true);
                        if let Some(gl) = gla.upgrade() { gl.grab_focus(); }
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        title_entry.add_controller(key_ctrl);

        // Focus-out → commit (e.g. user clicks a pane without pressing Enter)
        let focus_ctrl = EventControllerFocus::new();
        focus_ctrl.connect_leave({
            let c = commit;
            move |_| c()
        });
        title_entry.add_controller(focus_ctrl);

        // Double-click the label → begin rename
        let dbl = GestureClick::new();
        dbl.set_button(gdk::BUTTON_PRIMARY);
        dbl.connect_pressed({
            let lbl = lbl_w;
            let ent = ent_w;
            move |gesture, n_press, _, _| {
                if n_press < 2 {
                    // Let single-clicks through to the notebook's tab-switch.
                    gesture.set_state(gtk4::EventSequenceState::Denied);
                    return;
                }
                let current = lbl.label().to_string();
                ent.set_text(&current);
                ent.set_width_chars((current.len() as i32).max(8));
                lbl.set_visible(false);
                ent.set_visible(true);
                ent.grab_focus();
                ent.select_region(0, -1);
            }
        });
        title_label.add_controller(dbl);
    }

    pane.is_focused.set(true);
    let tab = Rc::new(Tab {
        container,
        panes: RefCell::new(vec![pane.clone()]),
        focused: RefCell::new(Some(pane.clone())),
        tab_label,
    });

    {
        let state = state.clone();
        let tab_w = Rc::downgrade(&tab);
        close_btn.connect_clicked(move |_| {
            let Some(t) = tab_w.upgrade() else { return };
            if state.confirm_tab_close.get() {
                let state = state.clone();
                confirm_close(
                    &state.window.clone(),
                    "Are you sure you want to close this tab?",
                    move || {
                        state.force_close.set(true);
                        close_tab(&state, &t);
                    },
                );
            } else {
                close_tab(&state, &t);
            }
        });
    }

    // Tab-strip drop target. While a pane is being dragged, hovering this
    // tab's label switches the notebook to this tab after a short delay so
    // the user can drop onto one of its panes. The drop itself is refused
    // (`DragAction::empty()` from `accept`); the user is expected to land on
    // a real pane below. Without the delay, brushing the cursor across the
    // tab strip would thrash through every tab it passes over.
    {
        let state_w = Rc::downgrade(&state);
        let tab_w = Rc::downgrade(&tab);
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let tab_drop = gtk4::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        // We never want a drop to *land* on the tab label, only to use enter
        // as a hover trigger. Reject any incoming drag value.
        tab_drop.connect_accept(|_, _| false);
        {
            let pending = pending.clone();
            let state_w = state_w.clone();
            let tab_w = tab_w.clone();
            tab_drop.connect_enter(move |_, _, _| {
                let (Some(state), Some(tab)) = (state_w.upgrade(), tab_w.upgrade())
                else {
                    return gdk::DragAction::empty();
                };
                if state.dragging.borrow().is_none() {
                    return gdk::DragAction::empty();
                }
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
                let state_w2 = Rc::downgrade(&state);
                let tab_w2 = Rc::downgrade(&tab);
                let pending2 = pending.clone();
                let id = glib::timeout_add_local_once(
                    Duration::from_millis(250),
                    move || {
                        *pending2.borrow_mut() = None;
                        let (Some(state), Some(tab)) =
                            (state_w2.upgrade(), tab_w2.upgrade())
                        else {
                            return;
                        };
                        if let Some(idx) = state.notebook.page_num(&tab.container) {
                            if state.notebook.current_page() != Some(idx) {
                                state.notebook.set_current_page(Some(idx));
                            }
                        }
                    },
                );
                *pending.borrow_mut() = Some(id);
                gdk::DragAction::empty()
            });
        }
        {
            let pending = pending.clone();
            tab_drop.connect_leave(move |_| {
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
            });
        }
        tab.tab_label.add_controller(tab_drop);
    }

    wire_pane(&state, &pane);
    Some(tab)
}

/// Close a tab and every pane inside it. If this was the last tab, close
/// the window. Clears `menu_target` if it pointed at any pane being removed.
fn close_tab(state: &Rc<WindowState>, tab: &Rc<Tab>) {
    // Clear any cached references into this tab's panes.
    let menu_target_in_tab = state
        .menu_target
        .borrow()
        .as_ref()
        .map(|t| tab.panes.borrow().iter().any(|p| Rc::ptr_eq(p, t)))
        .unwrap_or(false);
    if menu_target_in_tab {
        *state.menu_target.borrow_mut() = None;
    }

    let Some(page_num) = state.notebook.page_num(&tab.container) else {
        return;
    };
    state.notebook.remove_page(Some(page_num));
    state.tabs.borrow_mut().retain(|t| !Rc::ptr_eq(t, tab));

    let remaining = state.tabs.borrow().len();
    if remaining == 0 {
        state.window.close();
        return;
    }
    state.notebook.set_show_tabs(remaining > 1);
    // Tab count just dropped — a tab that previously showed its toolbar only
    // because there were other tabs may now collapse it.
    update_all_pane_toolbars(state);
}

/// Write the embedded `skyterm.svg` to the user's icon theme search path and
/// register that path with the active `IconTheme`, so windows that ask for
/// `icon_name = "skyterm"` find it. We write to `$XDG_DATA_HOME/icons` (or
/// `~/.local/share/icons`) so GTK4's default theme picks it up too.
///
/// All errors are silenced — a missing icon is cosmetic, not fatal.
fn install_app_icon() {
    const ICON_SVG: &str = include_str!("../resources/skyterm.svg");

    let Some(data_root) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    else {
        return;
    };
    let icon_root = data_root.join("icons");
    let icon_dir = icon_root.join("hicolor/scalable/apps");
    if std::fs::create_dir_all(&icon_dir).is_err() {
        return;
    }
    let target = icon_dir.join("skyterm.svg");
    // Only rewrite if the file is missing or stale — avoids touching the
    // mtime on every launch.
    let needs_write = std::fs::read_to_string(&target)
        .map(|existing| existing != ICON_SVG)
        .unwrap_or(true);
    if needs_write {
        let _ = std::fs::write(&target, ICON_SVG);
    }

    // Belt-and-braces: explicitly register the icon directory so we don't
    // depend on the OS having scanned ~/.local/share/icons recently.
    if let Some(display) = gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        theme.add_search_path(&icon_root);
    }
}

/// Substitute the configured terminal font's family name into [`CSS_TEMPLATE`]
/// so the menu / banner / split buttons render in the same face as the grid.
fn build_css(family: &str) -> String {
    // CSS strings are quoted to handle multi-word names ("DejaVu Sans Mono").
    let quoted = format!("\"{}\"", family.replace('"', ""));
    CSS_TEMPLATE.replace("__FONT_FAMILY__", &quoted)
}

/// Create the chrome CSS provider, register it with the default display, and
/// load it with the initial font family. The provider is returned so it can
/// be re-loaded later (see [`apply_chrome_font`]) when the user picks a
/// different font in Settings.
fn install_css(family: &str) -> CssProvider {
    let provider = CssProvider::new();
    provider.load_from_string(&build_css(family));
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    provider
}

/// Re-substitute the font family into the chrome stylesheet. Cheap — just
/// reloads the provider's string source; no extra `add_provider` call needed,
/// since the same provider remains registered with the display.
fn apply_chrome_font(state: &Rc<WindowState>) {
    let path = state.font_path.borrow().clone();
    let family = font::family_name(&path).unwrap_or_else(|_| "monospace".to_string());
    state.css_provider.load_from_string(&build_css(&family));
}

/// Build a fresh pane: spawn a PTY, build a GLArea + scrollbar, hook up the
/// GL realize/render callbacks, and start a PTY read loop. The pane is *not*
/// yet wired for input (keyboard, click, scroll, resize) — see [`wire_pane`].
fn make_pane(state: Rc<WindowState>, cols: u16, rows: u16) -> Option<Rc<Pane>> {
    let font_path = state.font_path.borrow().clone();
    let atlas = match font::build_atlas(&font_path, DEFAULT_FONT_SIZE_PX) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("font atlas build failed: {e}");
            return None;
        }
    };
    let cell_w = atlas.cell_w;
    let cell_h = atlas.cell_h;
    let atlas = Rc::new(atlas);

    let gl_area = GLArea::builder()
        .has_stencil_buffer(false)
        .has_depth_buffer(false)
        .hexpand(true)
        .vexpand(true)
        .build();
    gl_area.set_allowed_apis(gdk::GLAPI::GL);
    gl_area.set_required_version(3, 3);
    gl_area.set_focusable(true);

    let scroll_adj = Adjustment::new(
        0.0,
        0.0,
        rows as f64,
        1.0,
        rows as f64,
        rows as f64,
    );
    let scrollbar = Scrollbar::new(Orientation::Vertical, Some(&scroll_adj));
    scrollbar.set_visible(false);

    let pty_handle = match pty::spawn(cols, rows) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("pty spawn: {e}");
            return None;
        }
    };
    let rx = pty_handle.rx.clone();

    // Floating toolbar with drag handle + close button. Pinned to top-right of
    // the pane via halign/valign. Hidden by default; `update_pane_toolbars`
    // shows it whenever the tab has more than one pane.
    let drag_btn = gtk4::Button::new();
    drag_btn.set_label("\u{22EF}"); // midline horizontal ellipsis
    drag_btn.set_tooltip_text(Some("Drag this pane to a new position"));
    drag_btn.set_focusable(false);
    drag_btn.set_can_focus(false);

    let close_btn = gtk4::Button::new();
    close_btn.set_label("\u{2715}"); // multiplication x
    close_btn.set_tooltip_text(Some("Close pane"));
    close_btn.set_focusable(false);
    close_btn.set_can_focus(false);
    close_btn.add_css_class("pane-toolbar-close");

    let toolbar = gtk4::Box::new(Orientation::Horizontal, 0);
    toolbar.add_css_class("pane-toolbar");
    toolbar.set_halign(gtk4::Align::End);
    toolbar.set_valign(gtk4::Align::Start);
    toolbar.append(&drag_btn);
    toolbar.append(&close_btn);
    toolbar.set_visible(false);

    // Hover toggles the `hovered` CSS class for the 10% → 100% opacity swap.
    let motion = gtk4::EventControllerMotion::new();
    {
        let toolbar_w = toolbar.downgrade();
        motion.connect_enter(move |_, _, _| {
            if let Some(t) = toolbar_w.upgrade() {
                t.add_css_class("hovered");
            }
        });
    }
    {
        let toolbar_w = toolbar.downgrade();
        motion.connect_leave(move |_| {
            if let Some(t) = toolbar_w.upgrade() {
                t.remove_css_class("hovered");
            }
        });
    }
    toolbar.add_controller(motion);

    // Drop-edge highlight (green half-pane rectangle, shown during a drag).
    // can_target=false so the highlight is transparent to pointer picking —
    // otherwise it'd block the `DropTarget` motion events on the gl_area
    // underneath and the cursor would "stick".
    let drop_highlight = gtk4::Box::new(Orientation::Horizontal, 0);
    drop_highlight.add_css_class("pane-drop-highlight");
    drop_highlight.set_visible(false);
    drop_highlight.set_can_target(false);
    drop_highlight.set_focusable(false);

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&gl_area));
    // Highlight first, toolbar second — toolbar must stay on top so the close
    // button is clickable even while a drop highlight is showing.
    overlay.add_overlay(&drop_highlight);
    overlay.add_overlay(&toolbar);
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);

    let inner = gtk4::Box::new(Orientation::Horizontal, 0);
    inner.append(&overlay);
    inner.append(&scrollbar);

    let wrap = gtk4::Box::new(Orientation::Vertical, 0);
    wrap.append(&inner);
    wrap.add_css_class("pane-wrap");
    wrap.set_hexpand(true);
    wrap.set_vexpand(true);

    let pane = Rc::new(Pane {
        wrap,
        gl_area,
        toolbar,
        drop_highlight,
        scrollbar,
        scroll_adj,
        scroll_syncing: Cell::new(false),
        grid: RefCell::new(Grid::new(cols as usize, rows as usize)),
        parser: RefCell::new(Parser::new()),
        writer: RefCell::new(pty_handle.writer),
        master: RefCell::new(pty_handle.master),
        renderer: RefCell::new(None),
        font_size: Cell::new(DEFAULT_FONT_SIZE_PX),
        cell_dims: Cell::new((cell_w, cell_h)),
        font_path: state.font_path.clone(),
        theme: Rc::new(RefCell::new(state.theme.borrow().clone())),
        selection: RefCell::new(None),
        is_focused: Cell::new(false),
        resize_source: RefCell::new(None),
        click_state: Cell::new((0, 0, 0, 0)),
        _child: RefCell::new(pty_handle.child),
    });

    {
        let state = state.clone();
        let pane_w = Rc::downgrade(&pane);
        close_btn.connect_clicked(move |_| {
            if let Some(p) = pane_w.upgrade() {
                request_close_pane(&state, &p);
            }
        });
    }

    // Drag source on the `⋯` toolbar button. The drag content is a placeholder
    // string ("skyterm-pane") only to satisfy GTK's content-type negotiation —
    // the actual source pane is held in `state.dragging` so drop targets can
    // recover the full `Rc<Pane>`. Prepare refuses to start the drag when the
    // pane is alone in its tab (nowhere to rearrange to).
    {
        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);
        {
            let state = state.clone();
            let pane_w = Rc::downgrade(&pane);
            drag_source.connect_prepare(move |_, _, _| {
                let p = pane_w.upgrade()?;
                let tab = tab_of_pane(&state, &p)?;
                // Refuse only when the entire window has nowhere to drop:
                // a single pane in a single tab. With 2+ tabs we still allow
                // a sole-pane drag because the user can cross tabs.
                if tab.panes.borrow().len() < 2 && state.tabs.borrow().len() < 2 {
                    return None;
                }
                *state.dragging.borrow_mut() = Some(p);
                Some(gdk::ContentProvider::for_value(&"skyterm-pane".to_value()))
            });
        }
        {
            let state = state.clone();
            drag_source.connect_drag_end(move |_, _, _| {
                // Clears even if the drop didn't fire (cancelled, escape, etc.)
                // so a future drag doesn't see stale state, and hides any
                // lingering drop highlight that DropTarget::leave didn't catch
                // (e.g. cancellation while the cursor was still over a target).
                *state.dragging.borrow_mut() = None;
                for tab in state.tabs.borrow().iter() {
                    for p in tab.panes.borrow().iter() {
                        p.drop_highlight.set_visible(false);
                    }
                }
            });
        }
        drag_btn.add_controller(drag_source);
    }

    // GL unrealize — drop the renderer *while the old context is still
    // current*. Splits unparent and reparent the pane's GLArea, which makes
    // GTK destroy and recreate its GdkGLContext. Each fresh context starts
    // its ID counters from 1; if we let the old Renderer drop later (during
    // the next realize), its `glDelete*` calls run against the new context
    // and clobber the new Renderer's just-allocated atlas/program/VAO IDs —
    // leaving a blank pane.
    {
        let pane_w = Rc::downgrade(&pane);
        pane.gl_area.connect_unrealize(move |area| {
            area.make_current();
            if let Some(p) = pane_w.upgrade() {
                *p.renderer.borrow_mut() = None;
            }
        });
    }

    // GL realize — build the renderer fresh, using the pane's *current*
    // font path + size. Re-realize happens after split-reparenting, which is
    // also when font/family changes from Settings would have updated the
    // shared font_path; building from current state ensures the freshly-
    // realized renderer uses the right font.
    {
        drop(atlas); // captured below via fresh build instead
        let pane_w = Rc::downgrade(&pane);
        pane.gl_area.connect_realize(move |area| {
            area.make_current();
            if let Some(err) = area.error() {
                eprintln!("skyterm: GLArea realize: {err}");
                return;
            }
            init_gl_loader();
            let gl = unsafe {
                glow::Context::from_loader_function(|s| epoxy::get_proc_addr(s))
            };
            let Some(p) = pane_w.upgrade() else { return };
            let path = p.font_path.borrow().clone();
            let size = p.font_size.get();
            // On HiDPI (macOS Retina, scale_factor=2) the GL framebuffer is
            // device-pixel-sized while area.width()/height() report logical
            // pixels. The atlas must be rasterized at device resolution so the
            // glyphs look right, and the renderer's viewport+grid math must use
            // device pixels too — otherwise GL only draws into the bottom-left
            // quadrant of the framebuffer and the prompt floats mid-screen.
            let scale = area.scale_factor().max(1) as u32;
            let atlas = match font::build_atlas(&path, size * scale) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("skyterm: atlas build on realize: {e}");
                    return;
                }
            };
            p.cell_dims.set((atlas.cell_w, atlas.cell_h));
            let mut renderer = match Renderer::new(gl, &atlas) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("skyterm: renderer init: {e}");
                    return;
                }
            };
            let w = area.width() * scale as i32;
            let h = area.height() * scale as i32;
            if w > 0 && h > 0 {
                renderer.resize(w, h);
            }
            *p.renderer.borrow_mut() = Some(renderer);
            p.gl_area.queue_render();
        });
    }

    // GL render. Sync viewport from the widget's current size on every draw
    // — connect_resize alone is fragile across reparenting (a realize that
    // follows installs a new renderer with viewport=0 until the next
    // resize signal).
    {
        let pane_w = Rc::downgrade(&pane);
        let cursor_blink = state.cursor_blink.clone();
        let blink_phase = state.blink_phase.clone();
        pane.gl_area.connect_render(move |area, _ctx| {
            if let Some(p) = pane_w.upgrade() {
                // Device pixels — see the matching comment in connect_realize.
                let scale = area.scale_factor().max(1);
                let w = area.width() * scale;
                let h = area.height() * scale;
                // NOTE: do NOT reflow the grid here. Reflow moves the cursor,
                // and doing it per-frame during a drag interleaves with the
                // shell's SIGWINCH prompt redraw, corrupting the screen. Reflow
                // is debounced in `schedule_reflow` and fires once the size
                // settles. The render callback only syncs the GL viewport.
                if let Some(r) = p.renderer.borrow_mut().as_mut() {
                    if w > 0 && h > 0 {
                        r.resize(w, h);
                    }
                    let sel = *p.selection.borrow();
                    // Blink only on the focused pane; unfocused panes show a
                    // static visible cursor so they don't distract the user.
                    let cursor_on = if p.is_focused.get() {
                        !cursor_blink.get() || blink_phase.get()
                    } else {
                        true
                    };
                    r.render(&p.grid.borrow(), sel, &p.theme.borrow(), cursor_on);
                }
            }
            glib::Propagation::Stop
        });
    }

    // PTY read loop — feeds bytes into this pane's parser, returns DSR/DA
    // responses, kicks a redraw. Holds only a Weak<Pane> so the loop exits
    // once the pane is dropped. When the channel closes (shell exited and the
    // reader thread hit EOF on the PTY master) the loop falls out and we close
    // the pane — otherwise typing into the dead PTY blocks the GLib main loop
    // when the master's tty buffer eventually fills up.
    {
        let pane_w = Rc::downgrade(&pane);
        let state_for_exit = state.clone();
        glib::spawn_future_local(async move {
            while let Ok(bytes) = rx.recv().await {
                let Some(p) = pane_w.upgrade() else {
                    return;
                };
                log::debug!("pty<- {:?}", DebugBytes(&bytes));
                let responses = {
                    let mut g = p.grid.borrow_mut();
                    let mut parser = p.parser.borrow_mut();
                    parser.advance(&mut g, &bytes);
                    parser.take_responses()
                };
                if !responses.is_empty() {
                    let mut w = p.writer.borrow_mut();
                    if let Err(e) = w.write_all(&responses) {
                        log::warn!("pty response write: {e}");
                    }
                    let _ = w.flush();
                }
                sync_scrollbar(&p);
                p.gl_area.queue_render();
            }
            // Channel closed → shell exited. Close this pane; close_pane folds
            // through to close_tab when this was the tab's last pane, which in
            // turn closes the window when it was the last tab.
            if let Some(p) = pane_w.upgrade() {
                log::info!("pty channel closed, closing pane");
                close_pane(&state_for_exit, &p);
            }
        });
    }

    Some(pane)
}

/// Resize the grid + PTY to fit a `(w, h)` pixel area, using the pane's cell
/// dimensions. No-op if the resulting column/row count is unchanged. Called
/// both from the GLArea `resize` signal and as a fallback from the render
/// callback (the signal can be missed across some allocation paths).
/// Debounce a reflow to fire once the widget size settles. A window drag emits
/// a burst of resize events; reflowing on each one moves the cursor and fires a
/// SIGWINCH, and the shell's prompt-redraw responses then interleave with the
/// next reflow and corrupt the screen. Collapsing the burst into a single
/// grid+PTY resize gives the shell exactly one clean redraw cycle.
fn schedule_reflow(p: &Rc<Pane>) {
    if let Some(id) = p.resize_source.borrow_mut().take() {
        id.remove();
    }
    let pane_w = Rc::downgrade(p);
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
        let Some(p) = pane_w.upgrade() else {
            return;
        };
        // Clear the stored handle first: this source is one-shot and about to
        // finish, so a later resize must not try to .remove() a dead id.
        p.resize_source.replace(None);
        // cell_dims is in device pixels (atlas rasterized at font_size * scale),
        // so the widget dimensions must be in device pixels too for the row/col
        // arithmetic to match what the renderer draws.
        let scale = p.gl_area.scale_factor().max(1);
        let (dw, dh) = (p.gl_area.width() * scale, p.gl_area.height() * scale);
        if reflow_to_pixels(&p, dw, dh) {
            sync_scrollbar(&p);
            p.gl_area.queue_render();
        }
    });
    p.resize_source.replace(Some(id));
}

fn reflow_to_pixels(p: &Rc<Pane>, w: i32, h: i32) -> bool {
    if w <= 0 || h <= 0 {
        return false;
    }
    let (cw, ch) = p.cell_dims.get();
    if cw == 0 || ch == 0 {
        return false;
    }
    let new_cols = ((w as u32 / cw).max(1)) as u16;
    let new_rows = ((h as u32 / ch).max(1)) as u16;
    let (cur_cols, cur_rows) = {
        let g = p.grid.borrow();
        (g.cols() as u16, g.rows() as u16)
    };
    if new_cols == cur_cols && new_rows == cur_rows {
        return false;
    }
    log::debug!(
        "reflow: {cur_cols}x{cur_rows} -> {new_cols}x{new_rows} (px {w}x{h}, cell {cw}x{ch})"
    );
    p.grid
        .borrow_mut()
        .resize(new_cols as usize, new_rows as usize);
    if let Err(e) = p.master.borrow_mut().resize(portable_pty::PtySize {
        cols: new_cols,
        rows: new_rows,
        pixel_width: w as u16,
        pixel_height: h as u16,
    }) {
        log::warn!("pty resize: {e}");
    }
    true
}

/// Hook a pane up to keyboard, mouse-wheel scroll, click-to-focus, middle-
/// click paste, and widget resize. Done as a separate step from `make_pane`
/// because handlers need an `Rc<WindowState>` clone.
fn wire_pane(state: &Rc<WindowState>, pane: &Rc<Pane>) {
    // Resize → reflow grid + PTY.
    // connect_resize is a GLArea-specific signal that fires from the render
    // cycle after size_allocate marks needs_resize. Window resizes reach here
    // reliably; GtkPaned handle drags are caught by notify::position on the
    // Paned (wired in split_pane) which calls queue_render() to force the
    // cycle to run.
    {
        let pane_w = Rc::downgrade(pane);
        pane.gl_area.connect_resize(move |area, w, h| {
            let Some(p) = pane_w.upgrade() else {
                return;
            };
            // The signal's (w, h) are unreliable across backends: on macOS GTK4
            // delivers logical pixels here while the actual GL framebuffer is
            // device-sized, which causes GL to draw into only one quadrant.
            // Compute device pixels explicitly from area.width() * scale_factor
            // so the viewport matches the framebuffer on every backend.
            let scale = area.scale_factor().max(1);
            let dw = area.width() * scale;
            let dh = area.height() * scale;
            log::debug!(
                "connect_resize: signal {w}x{h}, logical {}x{}, scale {scale}, device {dw}x{dh}",
                area.width(), area.height()
            );
            if let Some(r) = p.renderer.borrow_mut().as_mut() {
                r.resize(dw, dh);
            }
            p.gl_area.queue_render();
            schedule_reflow(&p);
        });
    }

    // Scrollbar drag → view offset.
    {
        let pane_w = Rc::downgrade(pane);
        pane.scroll_adj.connect_value_changed(move |a| {
            let Some(p) = pane_w.upgrade() else {
                return;
            };
            if p.scroll_syncing.get() {
                return;
            }
            let sb = p.grid.borrow().scrollback_len();
            let value = a.value().max(0.0);
            let target_offset = (sb as f64 - value).round().max(0.0) as usize;
            p.grid.borrow_mut().set_view_offset(target_offset);
            p.gl_area.queue_render();
        });
    }

    // Mouse wheel — modifier+wheel zooms the *focused* pane; plain wheel
    // either forwards scroll events to the PTY (when the app has enabled mouse
    // reporting) or drives the scrollback view. On macOS the zoom modifier is
    // ⌘ (META) so it matches the rest of the app shortcuts and stays out of
    // the shell's way (Ctrl is the shell's modifier on Mac). On Linux/Windows
    // it's Ctrl, matching gnome-terminal / iTerm-on-Linux conventions.
    {
        let state = state.clone();
        let pane_w = Rc::downgrade(pane);
        let scroll_ctrl = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll_ctrl.connect_scroll(move |ctrl, _dx, dy| {
            let mods = ctrl.current_event_state();
            let zoom_mod = if cfg!(target_os = "macos") {
                gdk::ModifierType::META_MASK
            } else {
                gdk::ModifierType::CONTROL_MASK
            };
            if mods.contains(zoom_mod) {
                if let Some(focused) = focused_pane(&state) {
                    if dy < 0.0 {
                        change_font_size(&focused, 1);
                    } else if dy > 0.0 {
                        change_font_size(&focused, -1);
                    }
                }
                return glib::Propagation::Stop;
            }
            let Some(p) = pane_w.upgrade() else {
                return glib::Propagation::Stop;
            };

            // When the foreground app has enabled mouse reporting, forward
            // scroll events as synthetic button-64/65 mouse events rather
            // than moving the scrollback view.
            {
                let grid = p.grid.borrow();
                if grid.mouse_mode != MouseMode::None {
                    // Repeat once per logical notch (dy is ±1.0 per notch).
                    let notches = dy.abs().ceil() as usize;
                    // button 64 = scroll up, 65 = scroll down
                    let btn: u8 = if dy < 0.0 { 64 } else { 65 };
                    // Report at cell 1,1 — position is irrelevant for pure
                    // scroll in most apps (vim, less, man).
                    let col: u16 = 1;
                    let row: u16 = 1;
                    let seq: Vec<u8> = if grid.mouse_sgr {
                        // SGR: CSI < btn ; col ; row M
                        format!("\x1b[<{btn};{col};{row}M").into_bytes()
                    } else {
                        // X10: CSI M <btn+32> <col+32> <row+32>
                        vec![0x1b, b'[', b'M', btn + 32, col as u8 + 32, row as u8 + 32]
                    };
                    drop(grid);
                    let mut w = p.writer.borrow_mut();
                    for _ in 0..notches {
                        let _ = w.write_all(&seq);
                    }
                    return glib::Propagation::Stop;
                }
            }

            // No mouse reporting active.
            // In alt screen (vim, less, man without mouse mode): convert wheel
            // to arrow key sequences so the app can handle scrolling itself.
            // In the normal screen: scroll the scrollback view.
            let in_alt = p.grid.borrow().is_alt_screen();
            if in_alt {
                let lines = (dy.abs() * 3.0).round().max(1.0) as usize;
                // CSI A = cursor up, CSI B = cursor down.
                let seq: &[u8] = if dy < 0.0 { b"\x1b[A" } else { b"\x1b[B" };
                let mut w = p.writer.borrow_mut();
                for _ in 0..lines {
                    let _ = w.write_all(seq);
                }
            } else {
                clear_selection(&p);
                let lines_per_notch = 3.0;
                let new_value = (p.scroll_adj.value() + dy * lines_per_notch)
                    .max(p.scroll_adj.lower())
                    .min((p.scroll_adj.upper() - p.scroll_adj.page_size()).max(0.0));
                p.scroll_adj.set_value(new_value);
            }
            glib::Propagation::Stop
        });
        pane.gl_area.add_controller(scroll_ctrl);
    }

    // Left-button handling — one gesture for both focus-on-press and
    // drag-to-select. Two separate gestures on the same button conflict in
    // GTK4: whichever claims the event sequence first locks the others out,
    // and GestureClick tends to win on press. So we use a single
    // GestureDrag and hook its inherited `begin` signal (fires on press,
    // before any motion threshold) to handle focus / selection clear.
    {
        let state = state.clone();
        let pane_w = Rc::downgrade(pane);
        let drag = GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        // When the foreground app has mouse reporting on, the left button is
        // forwarded to it (so htop tabs / rows, vim, etc. are clickable).
        // Holding Shift bypasses reporting and does our local text selection —
        // the standard xterm escape hatch for copying out of a mouse-aware app.
        // (`forwards_mouse` is a free fn so all three closures can call it.)
        {
            let state = state.clone();
            let pane_w = pane_w.clone();
            drag.connect_begin(move |g, seq| {
                let Some(p) = pane_w.upgrade() else { return };
                focus_pane(&state, &p);
                if forwards_mouse(&p, g) {
                    if let Some((x, y)) = g.point(seq) {
                        if let Some((row, col)) = pixel_to_cell(&p, x, y) {
                            send_mouse(&p, 0, MouseAction::Press, col, row);
                        }
                    }
                    return;
                }
                // Local press. When word/line selection is enabled, count
                // consecutive clicks on the same cell: 1 = clear (start a fresh
                // drag), 2 = word, 3 = line, then cycle. Without a separate
                // GestureClick (which would conflict with this GestureDrag), we
                // derive the count from the event timestamp + cell ourselves.
                let click = g.point(seq).and_then(|(x, y)| pixel_to_cell(&p, x, y));
                let Some((row, col)) = click else {
                    clear_selection(&p);
                    return;
                };
                if !state.click_word_select.get() {
                    clear_selection(&p);
                    return;
                }
                let now = g.current_event_time();
                let dbl_ms = gtk4::Settings::default()
                    .map(|s| s.gtk_double_click_time() as u32)
                    .unwrap_or(400);
                let (last_t, last_r, last_c, count) = p.click_state.get();
                let consecutive =
                    last_r == row && last_c == col && now.wrapping_sub(last_t) <= dbl_ms;
                let count = if consecutive { count + 1 } else { 1 };
                p.click_state.set((now, row, col, count));
                match (count - 1) % 3 {
                    1 => select_word(&p, row, col),
                    2 => select_line(&p, row),
                    _ => clear_selection(&p),
                }
            });
        }
        {
            let pane_w = pane_w.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                let Some(p) = pane_w.upgrade() else { return };
                let Some((sx, sy)) = g.start_point() else { return };
                if forwards_mouse(&p, g) {
                    // Report drag motion only when the app asked for it
                    // (?1002 button-event / ?1003 any-event tracking).
                    let mode = p.grid.borrow().mouse_mode;
                    if matches!(mode, MouseMode::ButtonMotion | MouseMode::AnyMotion) {
                        if let Some((row, col)) = pixel_to_cell(&p, sx + dx, sy + dy) {
                            send_mouse(&p, 0, MouseAction::Motion, col, row);
                        }
                    }
                    return;
                }
                let mut sel = p.selection.borrow_mut();
                if sel.is_none() {
                    // First motion: anchor at the press origin.
                    if let Some(anchor) = pixel_to_cell(&p, sx, sy) {
                        *sel = Some(Selection { anchor, active: anchor });
                    }
                }
                if let Some(active_cell) = pixel_to_cell(&p, sx + dx, sy + dy) {
                    if let Some(s) = sel.as_mut() {
                        s.active = active_cell;
                    }
                }
                drop(sel);
                p.gl_area.queue_render();
            });
        }
        {
            let state = state.clone();
            let pane_w = pane_w.clone();
            drag.connect_end(move |g, seq| {
                let Some(p) = pane_w.upgrade() else { return };
                if forwards_mouse(&p, g) {
                    if let Some((x, y)) = g.point(seq) {
                        if let Some((row, col)) = pixel_to_cell(&p, x, y) {
                            send_mouse(&p, 0, MouseAction::Release, col, row);
                        }
                    }
                    return;
                }
                // Copy-on-select: on release of a word/line/drag selection,
                // push it to the clipboard automatically (when enabled).
                if state.copy_on_select.get() && p.selection.borrow().is_some() {
                    copy_selection(&p);
                }
            });
        }
        pane.gl_area.add_controller(drag);
    }

    // Drop target for pane rearrangement. The DragSource lives on the `⋯`
    // toolbar button of some *other* pane; this target receives motion/drop
    // events when that drag enters this pane's GLArea. Motion picks an edge
    // by 4-quadrant diagonal split and lights up the green highlight; drop
    // restructures the Paned tree so the source becomes a new split on that
    // edge of this pane.
    {
        let state = state.clone();
        let pane_w = Rc::downgrade(pane);
        let drop_target = gtk4::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        {
            let state = state.clone();
            let pane_w = pane_w.clone();
            drop_target.connect_motion(move |_, x, y| {
                let Some(p) = pane_w.upgrade() else {
                    return gdk::DragAction::empty();
                };
                let src = state.dragging.borrow().clone();
                let Some(src) = src else {
                    return gdk::DragAction::empty();
                };
                // Refuse self-drops; cross-tab drops are now supported (the
                // user crosses tabs via the tab-strip hover target).
                if Rc::ptr_eq(&p, &src) {
                    p.drop_highlight.set_visible(false);
                    return gdk::DragAction::empty();
                }
                let dir = compute_drop_dir(&p, x, y);
                show_drop_highlight(&p, dir);
                gdk::DragAction::MOVE
            });
        }
        {
            let pane_w = pane_w.clone();
            drop_target.connect_leave(move |_| {
                if let Some(p) = pane_w.upgrade() {
                    p.drop_highlight.set_visible(false);
                }
            });
        }
        {
            let state = state.clone();
            let pane_w = pane_w.clone();
            drop_target.connect_drop(move |_, _value, x, y| {
                let Some(p) = pane_w.upgrade() else { return false };
                p.drop_highlight.set_visible(false);
                let src = state.dragging.borrow_mut().take();
                let Some(src) = src else { return false };
                if Rc::ptr_eq(&p, &src) {
                    return false;
                }
                let dir = compute_drop_dir(&p, x, y);
                rearrange_pane(&state, &src, &p, dir);
                true
            });
        }
        pane.gl_area.add_controller(drop_target);
    }

    // Middle-click → primary-selection paste.
    {
        let pane_w = Rc::downgrade(pane);
        let middle = GestureClick::new();
        middle.set_button(gdk::BUTTON_MIDDLE);
        middle.connect_pressed(move |_, _, _, _| {
            if let Some(p) = pane_w.upgrade() {
                paste(&p, true);
            }
        });
        pane.gl_area.add_controller(middle);
    }

    // Right-click → context menu with split options. The popover is parented
    // to the toplevel window rather than the pane's GLArea: GTK4 sizes
    // popovers within their parent widget's allocation, and small panes
    // would otherwise produce squashed menus with scrollbars.
    let popover = PopoverMenu::from_model(Some(&state.split_menu));
    popover.set_parent(&state.window);
    popover.set_has_arrow(false);
    // Enforce a minimum width wide enough for the longest theme name
    // ("Skyterm Dracula" / "Solarized Dark" at ~14 chars in Fira Code 14px)
    // plus button padding. Height is unconstrained (-1); content drives it.
    popover.set_size_request(240, -1);
    // PopoverMenu wraps its content in an internal GtkScrolledWindow whose
    // default max-content-height is small enough that our 6-item menu
    // overflows and gets a scrollbar. Override on every show — the inner
    // widgets are created lazily so we can't catch them at construction.
    popover.connect_show(|p| {
        unconstrain_scrolled_windows(p.upcast_ref::<gtk4::Widget>());
    });
    // `visible-submenu` fires synchronously on GTK4 4.8+; belt-and-suspenders
    // for versions where it works.
    popover.connect_notify_local(Some("visible-submenu"), |p, _| {
        unconstrain_scrolled_windows(p.upcast_ref::<gtk4::Widget>());
        p.queue_resize();
    });
    // When the stack switches to a submenu page, GTK4 maps that page's
    // children. Connect `map` on every ScrolledWindow found at realize time
    // so that when any SW becomes visible we re-apply unconstrain and force a
    // resize. This catches submenu pages whether they're built upfront or lazily.
    popover.connect_realize(|p| {
        hook_sw_map_signals(p.upcast_ref::<gtk4::Widget>(), p.downgrade());
    });
    // Custom widget for the menu's "close-pane" slot. The standard menu
    // model can't style individual items, so we hand-render the destructive
    // action as a button with our `pane-close-btn` CSS class.
    {
        let close_btn = gtk4::Button::builder()
            .label("✕  Close pane")
            .css_classes(["pane-close-btn"])
            .build();
        let popover_w = popover.clone();
        close_btn.connect_clicked(move |btn| {
            popover_w.popdown();
            // The action lives on the window's "pane" action group; walking
            // up from the button finds it.
            let _ = btn.activate_action("pane.close", None);
        });
        popover.add_child(&close_btn, "close-pane");
    }
    {
        let state = state.clone();
        let pane_w = Rc::downgrade(pane);
        let popover = popover.clone();
        let right = GestureClick::new();
        right.set_button(gdk::BUTTON_SECONDARY);
        right.connect_pressed(move |_, _n, x, y| {
            let Some(p) = pane_w.upgrade() else {
                return;
            };
            // Right-click also focuses the pane — matches the user's mental
            // model of "the menu acts on the thing I clicked on".
            focus_pane(&state, &p);
            // The popover lives on the window; translate the click point
            // from gl_area coords into window coords for pointing_to.
            let point = gtk4::graphene::Point::new(x as f32, y as f32);
            let (px, py) = p
                .gl_area
                .compute_point(&state.window, &point)
                .map(|p| (p.x() as f64, p.y() as f64))
                .unwrap_or((x, y));
            *state.menu_target.borrow_mut() = Some(p);
            // Swap in the menu variant that matches the current state: hide
            // "Close pane" only when closing it would empty the entire
            // window (i.e. last pane in last tab). With multiple tabs, the
            // close action just closes the tab.
            let tabs = state.tabs.borrow().len();
            let panes_here = current_tab(&state)
                .map(|t| t.panes.borrow().len())
                .unwrap_or(0);
            let model = if tabs > 1 || panes_here > 1 {
                &state.split_menu
            } else {
                &state.split_menu_no_close
            };
            popover.set_menu_model(Some(model));
            let rect = gdk::Rectangle::new(px as i32, py as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.popup();
        });
        pane.gl_area.add_controller(right);
    }

    // Keyboard.
    let key_controller = EventControllerKey::new();
    {
        let state = state.clone();
        let pane_w = Rc::downgrade(pane);
        key_controller.connect_key_pressed(move |_, keyval, _keycode, modifiers| {
            let Some(p) = pane_w.upgrade() else {
                return glib::Propagation::Stop;
            };

            // macOS: ⌘-direct shortcuts (iTerm2-style). Ctrl is reserved for the
            // shell on Mac — Ctrl+A is readline's beginning-of-line, Ctrl+C is
            // SIGINT, Ctrl+V is literal-quote — so we put app actions on ⌘
            // (META_MASK) instead. The Ctrl+A chord prefix is suppressed below
            // on macOS for the same reason.
            if cfg!(target_os = "macos")
                && modifiers.contains(gdk::ModifierType::META_MASK)
            {
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
                let target = focused_pane(&state).unwrap_or_else(|| p.clone());

                // ⌘D  vertical split (new pane to the right)
                // ⌘⇧D horizontal split (new pane below)
                if matches!(keyval, gdk::Key::d | gdk::Key::D) {
                    let dir = if shift { SplitDir::Down } else { SplitDir::Right };
                    split(&state, &target, dir);
                    return glib::Propagation::Stop;
                }
                // ⌘T new tab
                if matches!(keyval, gdk::Key::t | gdk::Key::T) {
                    new_tab(&state);
                    return glib::Propagation::Stop;
                }
                // ⌘O cycle focus to the next pane in this tab
                if matches!(keyval, gdk::Key::o | gdk::Key::O) {
                    focus_next_pane(&state);
                    return glib::Propagation::Stop;
                }
                // ⌘⌥arrows: focus the pane in that direction
                if alt {
                    let focus_dir = match keyval {
                        gdk::Key::Up => Some(SplitDir::Up),
                        gdk::Key::Down => Some(SplitDir::Down),
                        gdk::Key::Left => Some(SplitDir::Left),
                        gdk::Key::Right => Some(SplitDir::Right),
                        _ => None,
                    };
                    if let Some(dir) = focus_dir {
                        focus_direction(&state, dir);
                        return glib::Propagation::Stop;
                    }
                }
                // ⌘C copy / ⌘V paste
                if matches!(keyval, gdk::Key::c | gdk::Key::C) {
                    copy_selection(&p);
                    return glib::Propagation::Stop;
                }
                if matches!(keyval, gdk::Key::v | gdk::Key::V) {
                    paste(&p, false);
                    return glib::Propagation::Stop;
                }
                // ⌘+ / ⌘= / ⌘- / ⌘0 font zoom
                match keyval {
                    gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                        change_font_size(&target, 1);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::minus | gdk::Key::KP_Subtract => {
                        change_font_size(&target, -1);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::_0 | gdk::Key::KP_0 => {
                        change_font_size(&target, 0);
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            // 1) Chord armed? Interpret the second key.
            let armed = state
                .chord_at
                .get()
                .map(|t| t.elapsed() < CHORD_TIMEOUT)
                .unwrap_or(false);
            if armed {
                // Modifier-only key events (e.g. user releasing/pressing Ctrl)
                // don't count — keep waiting.
                if is_modifier_only(keyval) {
                    return glib::Propagation::Stop;
                }
                state.chord_at.set(None);
                // Prefix-twice → pass literal Ctrl+A (0x01) through to the
                // PTY. Matches tmux's `send-prefix`.
                if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                    && matches!(keyval, gdk::Key::a | gdk::Key::A)
                {
                    snap_to_bottom(&p);
                    let mut w = p.writer.borrow_mut();
                    let _ = w.write_all(&[0x01]);
                    let _ = w.flush();
                    return glib::Propagation::Stop;
                }
                // `t` opens a new tab.
                if matches!(keyval, gdk::Key::t | gdk::Key::T) {
                    new_tab(&state);
                    return glib::Propagation::Stop;
                }
                // `n` opens a new top-level window.
                if matches!(keyval, gdk::Key::n | gdk::Key::N) {
                    if let Some(app) = state.window.application() {
                        on_activate(&app);
                    }
                    return glib::Propagation::Stop;
                }
                // `o` cycles focus to the next pane in the current tab.
                if matches!(keyval, gdk::Key::o | gdk::Key::O) {
                    focus_next_pane(&state);
                    return glib::Propagation::Stop;
                }
                // h/j/k/l move focus to the spatially-closest pane in that
                // direction (vim-style; arrows are reserved for splitting).
                let focus_dir = match keyval {
                    gdk::Key::h | gdk::Key::H => Some(SplitDir::Left),
                    gdk::Key::j | gdk::Key::J => Some(SplitDir::Down),
                    gdk::Key::k | gdk::Key::K => Some(SplitDir::Up),
                    gdk::Key::l | gdk::Key::L => Some(SplitDir::Right),
                    _ => None,
                };
                if let Some(dir) = focus_dir {
                    focus_direction(&state, dir);
                    return glib::Propagation::Stop;
                }
                // ' (apostrophe) — previous theme; / (slash) — next theme.
                // Wraps around the combined built-in + user theme list and
                // persists the new selection to config. The chord is *re-armed*
                // after firing so the user can rapid-scroll by holding Ctrl+A
                // and tapping ' / / repeatedly without having to hit the full
                // 3-key combo every time. Any non-cycle key (or the 2s timeout
                // expiring) disarms the chord normally.
                let delta = match keyval {
                    gdk::Key::apostrophe => Some(-1),
                    gdk::Key::slash => Some(1),
                    _ => None,
                };
                if let Some(d) = delta {
                    cycle_theme(&state, &p, d);
                    state.chord_at.set(Some(Instant::now()));
                    return glib::Propagation::Stop;
                }
                let dir = match keyval {
                    gdk::Key::Up => Some(SplitDir::Up),
                    gdk::Key::Down => Some(SplitDir::Down),
                    gdk::Key::Left => Some(SplitDir::Left),
                    gdk::Key::Right => Some(SplitDir::Right),
                    _ => None,
                };
                if let Some(dir) = dir {
                    split(&state, &p, dir);
                }
                // Swallow any unrecognized key after the prefix.
                return glib::Propagation::Stop;
            }

            // 2) Prefix: Ctrl+A (Linux/Windows) or ⌘A (macOS) — arm chord,
            // swallow. We use ⌘ on Mac instead of Ctrl so the shell's readline
            // beginning-of-line on Ctrl+A keeps working. ⌘A's usual Mac
            // meaning (Select All) is unused inside a terminal grid, so we
            // appropriate it as the chord prefix to mirror Linux's UX.
            let chord_mod = if cfg!(target_os = "macos") {
                gdk::ModifierType::META_MASK
            } else {
                gdk::ModifierType::CONTROL_MASK
            };
            if modifiers.contains(chord_mod)
                && matches!(keyval, gdk::Key::a | gdk::Key::A)
            {
                state.chord_at.set(Some(Instant::now()));
                return glib::Propagation::Stop;
            }

            // 3) Ctrl+Shift+C — copy the current selection to the clipboard.
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                && matches!(keyval, gdk::Key::c | gdk::Key::C)
            {
                copy_selection(&p);
                return glib::Propagation::Stop;
            }

            // 4) Ctrl+Shift+V — paste from system clipboard.
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                && matches!(keyval, gdk::Key::v | gdk::Key::V)
            {
                paste(&p, false);
                return glib::Propagation::Stop;
            }

            // 5) Ctrl+± font zoom on the focused pane.
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                let target = focused_pane(&state).unwrap_or_else(|| p.clone());
                match keyval {
                    gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                        change_font_size(&target, 1);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::minus | gdk::Key::KP_Subtract => {
                        change_font_size(&target, -1);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::_0 | gdk::Key::KP_0 => {
                        change_font_size(&target, 0);
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            // 6) Default: encode the keystroke and write to the PTY. Typing
            // input also clears the selection — the user has clearly moved
            // on from "I'm picking text to copy" — and snaps the view back
            // to the live screen if they were scrolled up.
            let app_cursor = p.grid.borrow().app_cursor_keys;
            let bytes = input::encode_key(keyval, modifiers, app_cursor);
            if !bytes.is_empty() {
                clear_selection(&p);
                snap_to_bottom(&p);
                let mut w = p.writer.borrow_mut();
                if let Err(e) = w.write_all(&bytes) {
                    log::warn!("pty write: {e}");
                }
                let _ = w.flush();
            }
            glib::Propagation::Stop
        });
    }
    pane.gl_area.add_controller(key_controller);
}

fn focus_pane(state: &Rc<WindowState>, pane: &Rc<Pane>) {
    let Some(tab) = tab_of_pane(state, pane) else {
        return;
    };
    // Swap the tab's focus highlight from the previous pane (if any in this
    // tab) to the new one.
    let prev = tab.focused.borrow().clone();
    if let Some(prev) = &prev {
        if Rc::ptr_eq(prev, pane) {
            pane.gl_area.grab_focus();
            return;
        }
        prev.wrap.remove_css_class("focused");
        prev.is_focused.set(false);
        prev.gl_area.queue_render();
    }
    pane.is_focused.set(true);
    pane.wrap.add_css_class("focused");
    *tab.focused.borrow_mut() = Some(pane.clone());

    // Bring the tab to the foreground if it isn't already.
    if let Some(page_num) = state.notebook.page_num(&tab.container) {
        if state.notebook.current_page() != Some(page_num) {
            state.notebook.set_current_page(Some(page_num));
        }
    }
    pane.gl_area.grab_focus();
}

/// Replace the focused pane's wrapper with a `GtkPaned` containing the old
/// pane + a freshly-spawned new pane, oriented per `dir`. The new pane takes
/// focus so chord chains continue against it.
fn split(state: &Rc<WindowState>, focused: &Rc<Pane>, dir: SplitDir) {
    let old_wrap = focused.wrap.clone();
    let Some(parent) = old_wrap.parent() else {
        log::warn!("split: focused pane has no parent");
        return;
    };

    // Inherit the focused pane's current grid size as a sensible initial size
    // for the new pane; resize on first allocation will correct it anyway.
    let (cols, rows) = {
        let g = focused.grid.borrow();
        (g.cols() as u16, g.rows() as u16)
    };
    let new_pane = match make_pane(state.clone(), cols.max(10), rows.max(3)) {
        Some(p) => p,
        None => return,
    };

    let orientation = match dir {
        SplitDir::Left | SplitDir::Right => Orientation::Horizontal,
        SplitDir::Up | SplitDir::Down => Orientation::Vertical,
    };
    let paned = Paned::new(orientation);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_hexpand(true);
    paned.set_vexpand(true);

    // Initial divider position — half of whatever the focused pane currently
    // occupies, along the split axis.
    let half = match orientation {
        Orientation::Horizontal => old_wrap.width() / 2,
        _ => old_wrap.height() / 2,
    };
    if half > 0 {
        paned.set_position(half);
    }

    // Detach `old_wrap` from its parent, slot the new `paned` in its place,
    // then put both wraps into the paned. The order matters: GTK4 widgets can
    // only have one parent, so we must unparent before re-parenting.
    let old_widget: gtk4::Widget = old_wrap.clone().upcast();
    let parent_box: Option<gtk4::Box> = parent.clone().downcast().ok();
    let parent_paned: Option<Paned> = parent.clone().downcast().ok();

    if let Some(b) = parent_box {
        b.remove(&old_wrap);
        b.append(&paned);
    } else if let Some(pp) = parent_paned {
        let in_start = pp
            .start_child()
            .map(|c| c == old_widget)
            .unwrap_or(false);
        if in_start {
            pp.set_start_child(gtk4::Widget::NONE);
            pp.set_start_child(Some(&paned));
        } else {
            pp.set_end_child(gtk4::Widget::NONE);
            pp.set_end_child(Some(&paned));
        }
    } else {
        log::warn!("split: unexpected parent widget type");
        return;
    }

    match dir {
        SplitDir::Right | SplitDir::Down => {
            paned.set_start_child(Some(&old_wrap));
            paned.set_end_child(Some(&new_pane.wrap));
        }
        SplitDir::Left | SplitDir::Up => {
            paned.set_start_child(Some(&new_pane.wrap));
            paned.set_end_child(Some(&old_wrap));
        }
    }

    // When the divider handle is dragged, GTK calls size_allocate on both
    // children but the GLArea's `resize` signal can be missed. Force a frame
    // and schedule a (debounced) reflow on both panes whenever the divider
    // position changes, so split panes rewrap to their new width too.
    {
        let old_w = Rc::downgrade(focused);
        let new_w = Rc::downgrade(&new_pane);
        paned.connect_notify_local(Some("position"), move |_, _| {
            if let Some(p) = old_w.upgrade() {
                p.gl_area.queue_render();
                schedule_reflow(&p);
            }
            if let Some(p) = new_w.upgrade() {
                p.gl_area.queue_render();
                schedule_reflow(&p);
            }
        });
    }

    // The new pane lives in the same tab as the focused one. (`tab_of_pane`
    // is cheap — a linear scan over a small tabs vec — and avoids threading
    // the tab through every callsite.)
    if let Some(tab) = tab_of_pane(state, focused) {
        tab.panes.borrow_mut().push(new_pane.clone());
        update_pane_toolbars(state, &tab);
    }
    wire_pane(state, &new_pane);
    focus_pane(state, &new_pane);
}

/// Close a pane, optionally behind the confirmation dialog when
/// `confirm_pane_close` is set. Used by the right-click menu's Close action
/// and the floating toolbar's X button.
fn request_close_pane(state: &Rc<WindowState>, pane: &Rc<Pane>) {
    if state.confirm_pane_close.get() {
        let state = state.clone();
        let pane = pane.clone();
        confirm_close(
            &state.window.clone(),
            "Are you sure you want to close this pane?",
            move || {
                state.force_close.set(true);
                close_pane(&state, &pane);
            },
        );
    } else {
        close_pane(state, pane);
    }
}

/// Pick the drop edge for a `(x, y)` pointer position inside a pane: the pane
/// is sliced into four triangles by its diagonals; whichever triangle the
/// cursor is in determines which edge the source pane snaps to. Matches
/// Terminator's drop-target feedback.
fn compute_drop_dir(pane: &Pane, x: f64, y: f64) -> SplitDir {
    let w = pane.gl_area.width() as f64;
    let h = pane.gl_area.height() as f64;
    // Normalize to (-0.5, 0.5); compare absolute distances from the center on
    // each axis so the larger one wins (= we're closer to that edge pair).
    let dx = if w > 0.0 { x / w - 0.5 } else { 0.0 };
    let dy = if h > 0.0 { y / h - 0.5 } else { 0.0 };
    if dx.abs() > dy.abs() {
        if dx < 0.0 { SplitDir::Left } else { SplitDir::Right }
    } else if dy < 0.0 {
        SplitDir::Up
    } else {
        SplitDir::Down
    }
}

/// Position and show the green drop-target highlight on `pane` for direction
/// `dir`. Covers half the pane along the appropriate axis.
///
/// Implementation note: rather than `valign/halign = Start/End` + a
/// `size_request`, this uses `Fill + Fill` with a one-sided margin that pushes
/// the opposite edge inward. `valign=End` + height_request was unreliable in
/// `GtkOverlay`'s child allocation (the bottom-half case rendered with zero
/// size on some GTK builds); Fill + margins is unambiguous and produces a
/// half-pane rectangle every time.
fn show_drop_highlight(pane: &Pane, dir: SplitDir) {
    let w = pane.gl_area.width();
    let h = pane.gl_area.height();
    let hl = &pane.drop_highlight;
    hl.set_halign(gtk4::Align::Fill);
    hl.set_valign(gtk4::Align::Fill);
    hl.set_size_request(-1, -1);
    // Clear any margin from the previous direction so transitions like
    // Up→Down don't leave both margin_top and margin_bottom set.
    hl.set_margin_top(0);
    hl.set_margin_bottom(0);
    hl.set_margin_start(0);
    hl.set_margin_end(0);
    match dir {
        SplitDir::Left => hl.set_margin_end((w / 2).max(0)),
        SplitDir::Right => hl.set_margin_start((w / 2).max(0)),
        SplitDir::Up => hl.set_margin_bottom((h / 2).max(0)),
        SplitDir::Down => hl.set_margin_top((h / 2).max(0)),
    }
    hl.set_visible(true);
}

/// Detach `pane.wrap` from its current parent so it can be re-attached
/// elsewhere. Two cases:
///
/// 1. Parent is a `Paned` (pane shares its tab with siblings) — collapse the
///    `Paned` so the sibling takes its place, as `close_pane` does.
/// 2. Parent is a `gtk4::Box` (pane is the sole pane in its tab) — just
///    remove it from the box; the tab is now empty and the caller is
///    responsible for closing it.
///
/// Leaves the `Pane` itself alive (doesn't touch the tab's `panes` vec or
/// focus). Returns true on success.
fn detach_pane_widget(pane: &Rc<Pane>) -> bool {
    let wrap = pane.wrap.clone();
    let Some(parent) = wrap.parent() else { return false };
    if let Ok(b) = parent.clone().downcast::<gtk4::Box>() {
        // Sole pane in its tab — just unparent.
        b.remove(&wrap);
        return true;
    }
    let Ok(parent_paned) = parent.downcast::<Paned>() else { return false };
    let wrap_widget: gtk4::Widget = wrap.clone().upcast();
    let in_start = parent_paned
        .start_child()
        .map(|c| c == wrap_widget)
        .unwrap_or(false);
    let sibling = if in_start {
        parent_paned.end_child()
    } else {
        parent_paned.start_child()
    };
    let Some(sibling) = sibling else { return false };
    let Some(grandparent) = parent_paned.parent() else { return false };

    // Detach both children of the dying Paned. wrap becomes orphaned, ready
    // for re-attachment by the caller.
    parent_paned.set_start_child(gtk4::Widget::NONE);
    parent_paned.set_end_child(gtk4::Widget::NONE);

    if let Ok(b) = grandparent.clone().downcast::<gtk4::Box>() {
        b.remove(&parent_paned);
        b.append(&sibling);
    } else if let Ok(gp) = grandparent.downcast::<Paned>() {
        let paned_widget: gtk4::Widget = parent_paned.clone().upcast();
        let gp_in_start = gp
            .start_child()
            .map(|c| c == paned_widget)
            .unwrap_or(false);
        if gp_in_start {
            gp.set_start_child(gtk4::Widget::NONE);
            gp.set_start_child(Some(&sibling));
        } else {
            gp.set_end_child(gtk4::Widget::NONE);
            gp.set_end_child(Some(&sibling));
        }
    } else {
        return false;
    }
    true
}

/// Insert `src.wrap` next to `dst.wrap` in the widget tree, oriented per
/// `dir`. Wraps both into a new `Paned` that replaces `dst.wrap` in its
/// current parent — mirroring the layout part of `split`, but using an
/// already-existing source pane rather than creating one.
fn attach_pane_next_to(src: &Rc<Pane>, dst: &Rc<Pane>, dir: SplitDir) -> bool {
    let dst_wrap = dst.wrap.clone();
    let Some(parent) = dst_wrap.parent() else { return false };
    let orientation = match dir {
        SplitDir::Left | SplitDir::Right => Orientation::Horizontal,
        SplitDir::Up | SplitDir::Down => Orientation::Vertical,
    };
    let paned = Paned::new(orientation);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_hexpand(true);
    paned.set_vexpand(true);

    let half = match orientation {
        Orientation::Horizontal => dst_wrap.width() / 2,
        _ => dst_wrap.height() / 2,
    };
    if half > 0 {
        paned.set_position(half);
    }

    let dst_widget: gtk4::Widget = dst_wrap.clone().upcast();
    if let Ok(b) = parent.clone().downcast::<gtk4::Box>() {
        b.remove(&dst_wrap);
        b.append(&paned);
    } else if let Ok(pp) = parent.downcast::<Paned>() {
        let in_start = pp
            .start_child()
            .map(|c| c == dst_widget)
            .unwrap_or(false);
        if in_start {
            pp.set_start_child(gtk4::Widget::NONE);
            pp.set_start_child(Some(&paned));
        } else {
            pp.set_end_child(gtk4::Widget::NONE);
            pp.set_end_child(Some(&paned));
        }
    } else {
        return false;
    }

    match dir {
        SplitDir::Right | SplitDir::Down => {
            paned.set_start_child(Some(&dst_wrap));
            paned.set_end_child(Some(&src.wrap));
        }
        SplitDir::Left | SplitDir::Up => {
            paned.set_start_child(Some(&src.wrap));
            paned.set_end_child(Some(&dst_wrap));
        }
    }

    // Reflow both panes when the new divider is dragged.
    let a_w = Rc::downgrade(src);
    let b_w = Rc::downgrade(dst);
    paned.connect_notify_local(Some("position"), move |_, _| {
        if let Some(p) = a_w.upgrade() {
            p.gl_area.queue_render();
            schedule_reflow(&p);
        }
        if let Some(p) = b_w.upgrade() {
            p.gl_area.queue_render();
            schedule_reflow(&p);
        }
    });
    true
}

/// Move `src` so it becomes a new split on edge `dir` of `dst`. Supports
/// both same-tab rearranges and cross-tab moves (drag a pane onto a different
/// tab, drop on one of its panes). No-op when the two panes are the same or
/// either is not currently tracked. If the source's tab ends up empty after
/// a cross-tab move, that tab is closed automatically.
fn rearrange_pane(state: &Rc<WindowState>, src: &Rc<Pane>, dst: &Rc<Pane>, dir: SplitDir) {
    if Rc::ptr_eq(src, dst) {
        return;
    }
    let Some(src_tab) = tab_of_pane(state, src) else { return };
    let Some(dst_tab) = tab_of_pane(state, dst) else { return };

    if !detach_pane_widget(src) {
        return;
    }
    if !attach_pane_next_to(src, dst, dir) {
        log::warn!("rearrange: attach failed after detach — pane is orphaned");
        return;
    }

    let cross_tab = !Rc::ptr_eq(&src_tab, &dst_tab);
    if cross_tab {
        // Move membership: out of src_tab's panes vec, into dst_tab's. If
        // src was src_tab's focused pane, promote the next remaining pane
        // (or `None`, which `focus_pane` below will sort out).
        src_tab.panes.borrow_mut().retain(|p| !Rc::ptr_eq(p, src));
        let src_was_focused = src_tab
            .focused
            .borrow()
            .as_ref()
            .map(|f| Rc::ptr_eq(f, src))
            .unwrap_or(false);
        if src_was_focused {
            let next = src_tab.panes.borrow().first().cloned();
            *src_tab.focused.borrow_mut() = next;
        }
        dst_tab.panes.borrow_mut().push(src.clone());

        if src_tab.panes.borrow().is_empty() {
            // Source tab is now empty — close it. `close_tab` already calls
            // `update_all_pane_toolbars`, which also covers dst_tab.
            close_tab(state, &src_tab);
        } else {
            update_pane_toolbars(state, &src_tab);
            update_pane_toolbars(state, &dst_tab);
        }
    }

    focus_pane(state, src);
}

/// Show the floating toolbar on every pane in a tab when there is more than
/// one pane in the tab, OR when there is more than one tab in the window —
/// the latter so a sole-pane tab still has a drag handle the user can grab
/// to move the pane across tabs. Hidden only on the "one pane, one tab" case
/// where there's nowhere to drag to and closing would empty the window.
fn update_pane_toolbars(state: &Rc<WindowState>, tab: &Rc<Tab>) {
    let tabs_count = state.tabs.borrow().len();
    let panes = tab.panes.borrow();
    let show = panes.len() > 1 || tabs_count > 1;
    for p in panes.iter() {
        p.toolbar.set_visible(show);
    }
}

/// Refresh toolbar visibility on every tab. Needed when the tab count itself
/// changes (new tab, close tab, cross-tab drag), since the per-tab decision
/// depends on the global tab count too.
fn update_all_pane_toolbars(state: &Rc<WindowState>) {
    for tab in state.tabs.borrow().iter() {
        update_pane_toolbars(state, tab);
    }
}

/// Tear down a pane: unhook it from the widget tree (collapsing its parent
/// `Paned` so the sibling takes the freed space), drop it from the registry
/// so its PTY/child go with the last `Rc`, and move focus to a remaining
/// pane. Closing the last pane is a no-op — the window stays open.
fn close_pane(state: &Rc<WindowState>, pane: &Rc<Pane>) {
    let wrap = pane.wrap.clone();
    let Some(parent) = wrap.parent() else {
        return;
    };
    let Some(tab) = tab_of_pane(state, pane) else {
        return;
    };

    // Case 1: parent is the tab's container `Box` → this is the only pane
    // in its tab. Close the whole tab (which will close the window if it's
    // also the last tab).
    if parent.downcast_ref::<gtk4::Box>().is_some() {
        close_tab(state, &tab);
        return;
    }

    // Case 2: parent is a `Paned`. Find the sibling, then replace the Paned
    // with the sibling in the grandparent.
    let Ok(parent_paned) = parent.clone().downcast::<Paned>() else {
        log::warn!("close: unexpected parent widget type");
        return;
    };
    let wrap_widget: gtk4::Widget = wrap.clone().upcast();
    let in_start = parent_paned
        .start_child()
        .map(|c| c == wrap_widget)
        .unwrap_or(false);
    let sibling = if in_start {
        parent_paned.end_child()
    } else {
        parent_paned.start_child()
    };
    let Some(sibling) = sibling else {
        log::warn!("close: paned has no sibling");
        return;
    };

    let Some(grandparent) = parent_paned.parent() else {
        log::warn!("close: paned has no grandparent");
        return;
    };

    // Detach both children of the dying Paned so we can move them freely.
    parent_paned.set_start_child(gtk4::Widget::NONE);
    parent_paned.set_end_child(gtk4::Widget::NONE);

    if let Ok(b) = grandparent.clone().downcast::<gtk4::Box>() {
        b.remove(&parent_paned);
        b.append(&sibling);
    } else if let Ok(gp) = grandparent.downcast::<Paned>() {
        let paned_widget: gtk4::Widget = parent_paned.clone().upcast();
        let gp_in_start = gp
            .start_child()
            .map(|c| c == paned_widget)
            .unwrap_or(false);
        if gp_in_start {
            gp.set_start_child(gtk4::Widget::NONE);
            gp.set_start_child(Some(&sibling));
        } else {
            gp.set_end_child(gtk4::Widget::NONE);
            gp.set_end_child(Some(&sibling));
        }
    } else {
        log::warn!("close: unexpected grandparent widget type");
        return;
    }

    // Drop the closing pane from its tab and clear any stale references to
    // it before we move focus.
    tab.panes.borrow_mut().retain(|p| !Rc::ptr_eq(p, pane));
    if state
        .menu_target
        .borrow()
        .as_ref()
        .map(|t| Rc::ptr_eq(t, pane))
        .unwrap_or(false)
    {
        *state.menu_target.borrow_mut() = None;
    }
    let was_focused = tab
        .focused
        .borrow()
        .as_ref()
        .map(|f| Rc::ptr_eq(f, pane))
        .unwrap_or(false);
    if was_focused {
        *tab.focused.borrow_mut() = None;
        let next = tab.panes.borrow().first().cloned();
        if let Some(p) = next {
            focus_pane(state, &p);
        }
    }
    update_pane_toolbars(state, &tab);
}

/// Characters that count as part of a "word" for double-click selection.
/// Beyond alphanumerics this includes the punctuation common in paths, URLs,
/// and identifiers, so double-clicking a path or flag grabs the whole token.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
        || matches!(
            ch,
            '_' | '-' | '.' | '/' | ':' | '~' | '@' | '%' | '+' | '=' | ',' | '#'
        )
}

/// Double-click: select the word under `(row, col)`. A word is a run of
/// `is_word_char` cells; clicking a non-word cell selects just that cell.
fn select_word(pane: &Pane, row: usize, col: usize) {
    let cols = pane.grid.borrow().cols();
    if cols == 0 || col >= cols {
        return;
    }
    let g = pane.grid.borrow();
    let (start, end) = if is_word_char(g.visible_cell(row, col).ch) {
        let mut s = col;
        while s > 0 && is_word_char(g.visible_cell(row, s - 1).ch) {
            s -= 1;
        }
        let mut e = col;
        while e + 1 < cols && is_word_char(g.visible_cell(row, e + 1).ch) {
            e += 1;
        }
        (s, e)
    } else {
        (col, col)
    };
    drop(g);
    *pane.selection.borrow_mut() = Some(Selection {
        anchor: (row, start),
        active: (row, end),
    });
    pane.gl_area.queue_render();
}

/// Triple-click: select the whole logical line through `row`, expanding across
/// soft-wrapped continuation rows and trimming trailing blanks for a clean copy.
fn select_line(pane: &Pane, row: usize) {
    let (rows, cols) = {
        let g = pane.grid.borrow();
        (g.rows(), g.cols())
    };
    if rows == 0 || cols == 0 || row >= rows {
        return;
    }
    let g = pane.grid.borrow();
    let mut start = row;
    while start > 0 && g.visible_row_wrapped(start - 1) {
        start -= 1;
    }
    let mut end = row;
    while end + 1 < rows && g.visible_row_wrapped(end) {
        end += 1;
    }
    let mut last_col = cols - 1;
    while last_col > 0 && g.visible_cell(end, last_col).ch == ' ' {
        last_col -= 1;
    }
    drop(g);
    *pane.selection.borrow_mut() = Some(Selection {
        anchor: (start, 0),
        active: (end, last_col),
    });
    pane.gl_area.queue_render();
}

/// Select the entire visible grid. After this, Copy grabs the whole pane.
fn select_all_pane(pane: &Pane) {
    let (rows, cols) = {
        let g = pane.grid.borrow();
        (g.rows(), g.cols())
    };
    if rows == 0 || cols == 0 {
        return;
    }
    *pane.selection.borrow_mut() = Some(Selection {
        anchor: (0, 0),
        active: (rows - 1, cols - 1),
    });
    pane.gl_area.queue_render();
}

/// Copy the current selection to the system clipboard. No-op if nothing is
/// selected. Trailing spaces per row are trimmed; rows are joined with `\n`.
fn copy_selection(pane: &Pane) {
    let text = {
        let Some(sel) = *pane.selection.borrow() else {
            return;
        };
        let ((sr, sc), (er, ec)) = sel.ordered();
        let g = pane.grid.borrow();
        let cols = g.cols();
        let rows = g.rows();
        if rows == 0 || cols == 0 {
            return;
        }
        let er = er.min(rows.saturating_sub(1));
        let ec = ec.min(cols.saturating_sub(1));
        let sr = sr.min(er);
        let sc = if sr == er { sc.min(ec) } else { sc.min(cols - 1) };

        let mut out = String::new();
        for r in sr..=er {
            let from = if r == sr { sc } else { 0 };
            let to = if r == er { ec } else { cols - 1 };
            let mut line = String::with_capacity(to - from + 1);
            for c in from..=to {
                line.push(g.visible_cell(r, c).ch);
            }
            // A soft-wrapped row's text continues on the next row, so join it
            // without a newline (and without trimming — it's full width). Only
            // hard line breaks become `\n`. Matches VTE copy behavior.
            let soft_wrap = r < er && to == cols - 1 && g.visible_row_wrapped(r);
            if soft_wrap {
                out.push_str(&line);
            } else {
                out.push_str(line.trim_end());
                if r < er {
                    out.push('\n');
                }
            }
        }
        out
    };
    if text.is_empty() {
        return;
    }
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_text(&text);
    }
}

/// Convert a widget-relative pixel position into the (view_row, col) cell
/// the click landed on. Returns `None` if cell dimensions or grid are zero.
/// Walk a widget subtree and reconfigure every `ScrolledWindow` to size to
/// its natural content height (no scrollbar, no truncation). Used to undo
/// `PopoverMenu`'s default behavior of wrapping items in a clipped scroller.
/// Walk the subtree rooted at `widget` and, for every `ScrolledWindow` found,
/// connect a `map` signal that re-applies `unconstrain_scrolled_windows` on
/// the whole popover and forces a resize. GTK4 maps a stack page's children
/// when that page becomes the visible child, so this catches submenu pages
/// that aren't yet the active page at realize/show time.
fn hook_sw_map_signals(widget: &gtk4::Widget, weak_pop: glib::WeakRef<PopoverMenu>) {
    if let Some(sw) = widget.downcast_ref::<gtk4::ScrolledWindow>() {
        let w = weak_pop.clone();
        sw.connect_map(move |_| {
            if let Some(p) = w.upgrade() {
                unconstrain_scrolled_windows(p.upcast_ref::<gtk4::Widget>());
                p.queue_resize();
            }
        });
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        hook_sw_map_signals(&c, weak_pop.clone());
        child = c.next_sibling();
    }
}

fn unconstrain_scrolled_windows(widget: &gtk4::Widget) {
    if let Some(sw) = widget.downcast_ref::<gtk4::ScrolledWindow>() {
        sw.set_propagate_natural_height(true);
        sw.set_propagate_natural_width(true);
        sw.set_max_content_height(-1);
        sw.set_max_content_width(-1);
        // Do NOT touch vscrollbar_policy / hscrollbar_policy. In GTK4 4.16+
        // the PopoverMenu uses its internal ScrolledWindow for the page-slide
        // animation; forcing Policy::Never breaks that and collapses the
        // popover when navigating to a submenu. Scrollbars are hidden via CSS
        // (`popover.menu scrollbar { display: none }`) instead.
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        unconstrain_scrolled_windows(&c);
        child = c.next_sibling();
    }
}

/// Whether a left-button gesture should be reported to the foreground app
/// rather than driving local text selection. True when the app enabled mouse
/// reporting and Shift is not held (Shift is the override for local select).
fn forwards_mouse(p: &Pane, g: &GestureDrag) -> bool {
    let shift = g
        .current_event_state()
        .contains(gdk::ModifierType::SHIFT_MASK);
    p.grid.borrow().mouse_mode != MouseMode::None && !shift
}

/// A mouse button event to report to the foreground app.
#[derive(Clone, Copy)]
enum MouseAction {
    Press,
    Release,
    /// Button held + pointer moved (drag), for ?1002/?1003.
    Motion,
}

/// Forward a mouse button event to the PTY in whichever protocol the app
/// enabled. `col`/`row` are 0-based view-relative cells; we convert to the
/// 1-based coordinates the wire format expects. `button` is 0=left, 1=middle,
/// 2=right (the scroll wheel uses 64/65 and goes through the scroll handler).
fn send_mouse(pane: &Pane, button: u8, action: MouseAction, col: usize, row: usize) {
    let sgr = pane.grid.borrow().mouse_sgr;
    let c = col as u32 + 1;
    let r = row as u32 + 1;
    let seq: Vec<u8> = if sgr {
        // SGR (?1006): CSI < b ; col ; row (M=press/motion, m=release).
        let mut b = button as u32;
        if matches!(action, MouseAction::Motion) {
            b += 32;
        }
        let final_ch = if matches!(action, MouseAction::Release) {
            'm'
        } else {
            'M'
        };
        format!("\x1b[<{b};{c};{r}{final_ch}").into_bytes()
    } else {
        // X10/normal: CSI M Cb Cx Cy, all byte-offset by 32. Release reports
        // button 3; coordinates clamp at 255 (the legacy format's ceiling).
        let mut b = match action {
            MouseAction::Release => 3u32,
            _ => button as u32,
        };
        if matches!(action, MouseAction::Motion) {
            b += 32;
        }
        let cb = (b + 32).min(255) as u8;
        let cx = (c + 32).min(255) as u8;
        let cy = (r + 32).min(255) as u8;
        vec![0x1b, b'[', b'M', cb, cx, cy]
    };
    let mut w = pane.writer.borrow_mut();
    let _ = w.write_all(&seq);
    let _ = w.flush();
}

fn pixel_to_cell(pane: &Pane, x: f64, y: f64) -> Option<(usize, usize)> {
    let (cw, ch) = pane.cell_dims.get();
    if cw == 0 || ch == 0 {
        return None;
    }
    // GTK gesture coordinates are logical pixels but cell_dims is in device
    // pixels (atlas rasterized at font_size * scale_factor). Convert.
    let scale = pane.gl_area.scale_factor().max(1) as f64;
    let col = ((x.max(0.0) * scale) as u32 / cw) as usize;
    let row = ((y.max(0.0) * scale) as u32 / ch) as usize;
    let g = pane.grid.borrow();
    if g.cols() == 0 || g.rows() == 0 {
        return None;
    }
    Some((row.min(g.rows() - 1), col.min(g.cols() - 1)))
}

/// Standard terminal behavior: when the user is viewing scrollback and
/// types or pastes, snap the view back to the live screen so they can see
/// what they're about to send. No-op when already at the bottom.
fn snap_to_bottom(pane: &Pane) {
    if pane.grid.borrow().view_offset() > 0 {
        pane.grid.borrow_mut().set_view_offset(0);
        sync_scrollbar(pane);
        pane.gl_area.queue_render();
    }
}

/// Clear the pane's selection and queue a redraw if there was anything to
/// clear. Safe to call when nothing is selected.
fn clear_selection(pane: &Pane) {
    if pane.selection.borrow().is_some() {
        *pane.selection.borrow_mut() = None;
        pane.gl_area.queue_render();
    }
}

fn change_font_size(pane: &Pane, delta: i32) {
    let current = pane.font_size.get();
    let target = if delta == 0 {
        DEFAULT_FONT_SIZE_PX
    } else {
        let next = current as i32 + delta;
        (next.max(MIN_FONT_SIZE_PX as i32).min(MAX_FONT_SIZE_PX as i32)) as u32
    };
    set_font_size(pane, target);
}

/// Resize a single pane's font to an absolute pixel size. Clamps to the
/// allowed range. Used by both keyboard zoom (via [`change_font_size`]) and
/// the Settings modal.
fn set_font_size(pane: &Pane, target: u32) {
    let target = target.clamp(MIN_FONT_SIZE_PX, MAX_FONT_SIZE_PX);
    if target == pane.font_size.get() {
        return;
    }
    let path = pane.font_path.borrow().clone();
    // Rasterize at device resolution — see the comment in connect_realize.
    let scale = pane.gl_area.scale_factor().max(1) as u32;
    let new_atlas = match font::build_atlas(&path, target * scale) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("font resize to {target}px failed: {e}");
            return;
        }
    };
    pane.font_size.set(target);
    pane.cell_dims.set((new_atlas.cell_w, new_atlas.cell_h));

    pane.gl_area.make_current();
    let viewport = {
        let mut r = pane.renderer.borrow_mut();
        if let Some(r) = r.as_mut() {
            r.set_atlas(&new_atlas);
            r.viewport()
        } else {
            (0, 0)
        }
    };

    let (vw, vh) = viewport;
    if vw > 0 && vh > 0 {
        let cw = new_atlas.cell_w;
        let ch = new_atlas.cell_h;
        let new_cols = (((vw as u32) / cw).max(1)) as u16;
        let new_rows = (((vh as u32) / ch).max(1)) as u16;
        let (cur_cols, cur_rows) = {
            let g = pane.grid.borrow();
            (g.cols() as u16, g.rows() as u16)
        };
        if new_cols != cur_cols || new_rows != cur_rows {
            pane.grid
                .borrow_mut()
                .resize(new_cols as usize, new_rows as usize);
            let r = pane.master.borrow_mut().resize(portable_pty::PtySize {
                cols: new_cols,
                rows: new_rows,
                pixel_width: vw as u16,
                pixel_height: vh as u16,
            });
            if let Err(e) = r {
                log::warn!("pty resize: {e}");
            }
        }
        sync_scrollbar(pane);
    }
    pane.gl_area.queue_render();
}

fn sync_scrollbar(pane: &Pane) {
    let g = pane.grid.borrow();
    let sb = g.scrollback_len();
    let rows = g.rows();
    let view_offset = g.view_offset().min(sb);
    let total = (sb + rows) as f64;
    let page = rows as f64;
    let value = (sb - view_offset) as f64;
    pane.scroll_syncing.set(true);
    pane.scroll_adj.set_upper(total);
    pane.scroll_adj.set_page_size(page);
    pane.scroll_adj.set_page_increment(page);
    pane.scroll_adj.set_step_increment(1.0);
    pane.scroll_adj.set_value(value);
    pane.scroll_syncing.set(false);
    pane.scrollbar.set_visible(sb > 0);
}

/// Walk `~/.config/skyterm/themes/` and parse every `*.toml` file inside as
/// a skyterm theme config (see [`parse_toml_themes`] for the expected
/// `[themes.Name]` table layout). Result is sorted alphabetically by theme
/// name for stable menu order. Files with no parseable themes are silently
/// skipped so a stray non-theme file in the directory can't break startup.
fn load_user_themes() -> Vec<Theme> {
    let Some(dir) = user_themes_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // Missing directory is the common case for fresh installs — quietly
        // return empty rather than spamming logs.
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Restrict to *.toml so unrelated files in the dir don't get parsed.
        let is_toml = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("toml"))
            .unwrap_or(false);
        if !is_toml {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.extend(skyterm_core::theme::parse_toml_themes(&text));
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    if !out.is_empty() {
        log::info!(
            "loaded {} user theme(s) from {}",
            out.len(),
            dir.display()
        );
    }
    out
}

fn user_themes_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("skyterm").join("themes"))
}

/// Apply a font size to every pane in every tab.
fn apply_font_size_all(state: &Rc<WindowState>, target: u32) {
    for tab in state.tabs.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            set_font_size(pane, target);
        }
    }
}

/// Swap font family globally — updates the shared font_path and rebuilds
/// every pane's atlas at its current size. No-op if the path is unchanged.
fn apply_font_family_all(state: &Rc<WindowState>, new_path: PathBuf) {
    if *state.font_path.borrow() == new_path {
        return;
    }
    *state.font_path.borrow_mut() = new_path.clone();
    // Reload the chrome stylesheet so popovers / banners pick up the new
    // family alongside the grid.
    apply_chrome_font(state);
    // Pane.font_path is a clone of the same Rc, so it already sees the new
    // value. We just need each pane to re-rasterize its atlas at the
    // current size.
    for tab in state.tabs.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            let size = pane.font_size.get();
            let scale = pane.gl_area.scale_factor().max(1) as u32;
            let Ok(atlas) = font::build_atlas(&new_path, size * scale) else {
                continue;
            };
            pane.cell_dims.set((atlas.cell_w, atlas.cell_h));
            pane.gl_area.make_current();
            if let Some(r) = pane.renderer.borrow_mut().as_mut() {
                r.set_atlas(&atlas);
            }
            pane.gl_area.queue_render();
        }
    }
}

/// Apply only the font-related fields a theme might carry (`font_path`,
/// `font_size`). Used by both the global Settings apply path and the
/// per-pane right-click apply path so a theme that bundles a font actually
/// swaps the font regardless of where the user picked it from. Fields that
/// the theme doesn't set are left alone. Invalid font paths fall through
/// silently via the atlas-build error path in `apply_font_family_all`.
fn apply_theme_font(state: &Rc<WindowState>, theme: &Theme) {
    if let Some(path) = theme.font_path.clone() {
        apply_font_family_all(state, path);
    }
    if let Some(size) = theme.font_size {
        apply_font_size_all(state, size);
    }
}

/// Swap the active color theme globally. `Cell` colors are palette references,
/// so existing text re-tints on the next render. Bundled font / size, if any,
/// are applied via [`apply_theme_font`] — `font_path` and `font_size` are
/// global, not per-pane.
fn apply_theme(state: &Rc<WindowState>, theme: Theme) {
    *state.theme.borrow_mut() = theme.clone();
    for tab in state.tabs.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            *pane.theme.borrow_mut() = theme.clone();
            pane.gl_area.queue_render();
        }
    }
    apply_theme_font(state, &theme);
}

/// Step through `available_themes` by `delta` (+1 next, -1 previous), wrapping
/// at the ends. Only the given pane is updated — the chord is per-pane so the
/// user can theme individual panes differently. Not persisted to config (config
/// holds the global default theme set from Settings).
fn cycle_theme(state: &Rc<WindowState>, pane: &Rc<Pane>, delta: isize) {
    let themes = &state.available_themes;
    if themes.is_empty() {
        return;
    }
    let current = pane.theme.borrow().name.clone();
    let idx = themes
        .iter()
        .position(|t| t.name == current)
        .unwrap_or(0) as isize;
    let len = themes.len() as isize;
    let next = ((idx + delta) % len + len) % len;
    let theme = themes[next as usize].clone();
    log::info!("cycle theme -> {} (pane)", theme.name);
    *pane.theme.borrow_mut() = theme;
    pane.gl_area.queue_render();
}

/// Set the maximum scrollback lines per pane.
fn apply_scrollback_all(state: &Rc<WindowState>, lines: usize) {
    for tab in state.tabs.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            pane.grid.borrow_mut().set_scrollback_max(lines);
        }
    }
}

/// Snapshot the current state into a `Config` and write to disk.
fn save_config(state: &Rc<WindowState>) {
    let Some(path) = skyterm_core::config::Config::default_path() else {
        return;
    };
    // Sample the focused pane (if any) for current font size / scrollback;
    // those don't live on the WindowState yet.
    let (font_size, scrollback) = focused_pane(state)
        .map(|p| {
            let s = p.font_size.get();
            let sb = p.grid.borrow().scrollback_max();
            (Some(s), Some(sb))
        })
        .unwrap_or((None, None));
    let cfg = skyterm_core::config::Config {
        font_path: Some(state.font_path.borrow().clone()),
        font_size,
        theme_name: Some(state.theme.borrow().name.clone()),
        scrollback_lines: scrollback,
        cursor_blink: Some(state.cursor_blink.get()),
        click_word_select: Some(state.click_word_select.get()),
        copy_on_select: Some(state.copy_on_select.get()),
        tab_max_number: Some(state.tab_max.get()),
        confirm_tab_close: Some(state.confirm_tab_close.get()),
        confirm_pane_close: Some(state.confirm_pane_close.get()),
        confirm_window_close: Some(state.confirm_window_close.get()),
    };
    if let Err(e) = cfg.save(&path) {
        log::warn!("save config: {e}");
    }
}

/// Open the Settings modal — Appearance / Theme / Behavior / Keybindings.
/// Every control applies live and writes back to the config file.
fn open_about(state: &Rc<WindowState>) {
    use gtk4::{Align, Label, LinkButton};

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const REPO: &str = env!("CARGO_PKG_REPOSITORY");

    let dialog = gtk4::Window::new();
    dialog.set_title(Some("About Skyterm"));
    dialog.set_transient_for(Some(&state.window));
    dialog.set_modal(true);
    dialog.set_resizable(false);
    dialog.set_default_size(320, -1);

    let vbox = gtk4::Box::new(Orientation::Vertical, 16);
    vbox.set_margin_top(32);
    vbox.set_margin_bottom(32);
    vbox.set_margin_start(32);
    vbox.set_margin_end(32);
    vbox.set_halign(Align::Center);

    let name_label = Label::new(Some("Skyterm"));
    name_label.add_css_class("title-1");
    name_label.set_halign(Align::Center);

    let version_label = Label::new(Some(&format!("Version {VERSION}")));
    version_label.add_css_class("dim-label");
    version_label.set_halign(Align::Center);

    let desc_label = Label::new(Some("GPU-rendered terminal emulator"));
    desc_label.set_halign(Align::Center);
    desc_label.set_wrap(true);

    let repo_btn = LinkButton::with_label(REPO, "GitHub Repository");
    repo_btn.set_halign(Align::Center);

    vbox.append(&name_label);
    vbox.append(&version_label);
    vbox.append(&desc_label);
    vbox.append(&repo_btn);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn open_settings(state: &Rc<WindowState>) {
    use gtk4::{Align, Button, DropDown, Grid as GtkGrid, Label, ListBox, ListBoxRow,
        ScrolledWindow, Separator, SpinButton, StringList};

    let dialog = gtk4::Window::new();
    dialog.set_title(Some("skyterm Settings"));
    dialog.set_transient_for(Some(&state.window));
    dialog.set_modal(true);
    dialog.set_default_size(560, 460);

    let outer = gtk4::Box::new(Orientation::Vertical, 0);
    let notebook = gtk4::Notebook::new();
    notebook.set_hexpand(true);
    notebook.set_vexpand(true);

    // ── Appearance ───────────────────────────────────────────────────
    let appearance = GtkGrid::new();
    appearance.set_row_spacing(10);
    appearance.set_column_spacing(14);
    appearance.set_margin_start(20);
    appearance.set_margin_end(20);
    appearance.set_margin_top(20);
    appearance.set_margin_bottom(20);

    let row_label = |t: &str| -> Label {
        Label::builder().label(t).xalign(0.0).build()
    };

    // Font family
    appearance.attach(&row_label("Font family"), 0, 0, 1, 1);
    let fonts = font::available_monospace_fonts();
    let font_names: Vec<&str> = fonts.iter().map(|(n, _)| n.as_str()).collect();
    let font_model = StringList::new(&font_names);
    let font_dd = DropDown::new(Some(font_model), gtk4::Expression::NONE);
    font_dd.set_hexpand(true);
    let current_path = state.font_path.borrow().clone();
    if let Some(idx) = fonts.iter().position(|(_, p)| *p == current_path) {
        font_dd.set_selected(idx as u32);
    }
    {
        let state = state.clone();
        let fonts = fonts.clone();
        font_dd.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            if let Some((_, path)) = fonts.get(idx) {
                apply_font_family_all(&state, path.clone());
                save_config(&state);
            }
        });
    }
    appearance.attach(&font_dd, 1, 0, 1, 1);

    // Font size
    appearance.attach(&row_label("Font size"), 0, 1, 1, 1);
    let size_spin = SpinButton::with_range(
        MIN_FONT_SIZE_PX as f64,
        MAX_FONT_SIZE_PX as f64,
        1.0,
    );
    let current_size = focused_pane(state)
        .map(|p| p.font_size.get())
        .unwrap_or(DEFAULT_FONT_SIZE_PX);
    size_spin.set_value(current_size as f64);
    size_spin.set_hexpand(true);
    {
        let state = state.clone();
        size_spin.connect_value_changed(move |s| {
            let v = s.value() as u32;
            apply_font_size_all(&state, v);
            save_config(&state);
        });
    }
    appearance.attach(&size_spin, 1, 1, 1, 1);

    notebook.append_page(&appearance, Some(&Label::new(Some("Appearance"))));

    // ── Theme ────────────────────────────────────────────────────────
    let theme_page = GtkGrid::new();
    theme_page.set_row_spacing(10);
    theme_page.set_column_spacing(14);
    theme_page.set_margin_start(20);
    theme_page.set_margin_end(20);
    theme_page.set_margin_top(20);
    theme_page.set_margin_bottom(20);

    let themes = state.available_themes.clone();
    let current_theme_name = state.theme.borrow().name.clone();

    let theme_list = ListBox::new();
    theme_list.set_selection_mode(gtk4::SelectionMode::Browse);

    // Helper: append a non-selectable section header row.
    let add_header = |list: &ListBox, text: &str| {
        let row = ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        let lbl = Label::builder().label(text).xalign(0.0).build();
        lbl.add_css_class("heading");
        lbl.set_margin_start(6);
        lbl.set_margin_top(6);
        lbl.set_margin_bottom(2);
        row.set_child(Some(&lbl));
        list.append(&row);
    };

    let mut selected_row: Option<ListBoxRow> = None;

    // Partition the themes list into three buckets — built-in dark, built-in
    // light, user-loaded "Custom" — while keeping each row's index into the
    // original `themes` vec (so the selection handler can still look up the
    // theme by `widget_name`).
    let mut dark_idxs: Vec<usize> = Vec::new();
    let mut light_idxs: Vec<usize> = Vec::new();
    let mut custom_idxs: Vec<usize> = Vec::new();
    for (idx, t) in themes.iter().enumerate() {
        if state.user_theme_names.contains(&t.name) {
            custom_idxs.push(idx);
        } else if t.is_dark() {
            dark_idxs.push(idx);
        } else {
            light_idxs.push(idx);
        }
    }

    let add_theme_row = |theme_list: &ListBox,
                         idx: usize,
                         t: &Theme,
                         selected_row: &mut Option<ListBoxRow>| {
        let row = ListBoxRow::new();
        row.set_widget_name(&idx.to_string());
        let lbl = Label::builder().label(t.name.as_str()).xalign(0.0).build();
        lbl.set_margin_start(14);
        lbl.set_margin_top(4);
        lbl.set_margin_bottom(4);
        row.set_child(Some(&lbl));
        if t.name == current_theme_name {
            *selected_row = Some(row.clone());
        }
        theme_list.append(&row);
    };
    let add_separator = |theme_list: &ListBox| {
        let sep_row = ListBoxRow::new();
        sep_row.set_selectable(false);
        sep_row.set_activatable(false);
        sep_row.set_child(Some(&Separator::new(Orientation::Horizontal)));
        theme_list.append(&sep_row);
    };

    let mut need_sep = false;
    for (label, idxs) in [
        ("Dark", &dark_idxs),
        ("Light", &light_idxs),
        ("Custom", &custom_idxs),
    ] {
        if idxs.is_empty() {
            continue;
        }
        if need_sep {
            add_separator(&theme_list);
        }
        add_header(&theme_list, label);
        for &idx in idxs {
            add_theme_row(&theme_list, idx, &themes[idx], &mut selected_row);
        }
        need_sep = true;
    }

    if let Some(r) = selected_row {
        theme_list.select_row(Some(&r));
    }

    {
        let state = state.clone();
        let themes = themes.clone();
        theme_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let Ok(idx) = row.widget_name().as_str().parse::<usize>() else { return };
            if let Some(t) = themes.get(idx) {
                apply_theme(&state, t.clone());
                save_config(&state);
            }
        });
    }

    let theme_scroll = ScrolledWindow::builder()
        .hexpand(true)
        .min_content_height(180)
        .max_content_height(240)
        .build();
    theme_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    theme_scroll.set_child(Some(&theme_list));

    theme_page.attach(&theme_scroll, 0, 0, 2, 1);

    let theme_note = Label::builder()
        .label(
            "Built-in themes plus any in ~/.config/skyterm/themes/ \
             (Terminator-format files). Themes retint existing text live.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    theme_note.add_css_class("dim-label");
    theme_page.attach(&theme_note, 0, 1, 2, 1);

    notebook.append_page(&theme_page, Some(&Label::new(Some("Theme"))));

    // ── Behavior ─────────────────────────────────────────────────────
    let behavior = GtkGrid::new();
    behavior.set_row_spacing(10);
    behavior.set_column_spacing(14);
    behavior.set_margin_start(20);
    behavior.set_margin_end(20);
    behavior.set_margin_top(20);
    behavior.set_margin_bottom(20);

    // Cursor blink toggle
    behavior.attach(&row_label("Cursor blink"), 0, 0, 1, 1);
    let blink_switch = gtk4::Switch::new();
    blink_switch.set_active(state.cursor_blink.get());
    blink_switch.set_halign(Align::Start);
    {
        let state = state.clone();
        blink_switch.connect_active_notify(move |sw| {
            let enabled = sw.is_active();
            state.cursor_blink.set(enabled);
            if !enabled {
                // Cursor off → snap phase to visible so it stays shown.
                state.blink_phase.set(true);
            }
            if let Some(tab) = current_tab(&state) {
                for p in tab.panes.borrow().iter() {
                    p.gl_area.queue_render();
                }
            }
            save_config(&state);
        });
    }
    behavior.attach(&blink_switch, 1, 0, 1, 1);

    behavior.attach(&row_label("Scrollback lines"), 0, 1, 1, 1);
    let scrollback_spin = SpinButton::with_range(0.0, 1_000_000.0, 500.0);
    let current_sb = focused_pane(state)
        .map(|p| p.grid.borrow().scrollback_max() as f64)
        .unwrap_or(10_000.0);
    scrollback_spin.set_value(current_sb);
    scrollback_spin.set_hexpand(true);
    scrollback_spin
        .set_tooltip_text(Some("Set to 0 for infinite scrollback (no line limit)."));
    {
        let state = state.clone();
        scrollback_spin.connect_value_changed(move |s| {
            apply_scrollback_all(&state, s.value() as usize);
            save_config(&state);
        });
    }
    behavior.attach(&scrollback_spin, 1, 1, 1, 1);

    let sb_note = Label::builder()
        .label("0 = infinite (no cap). Memory grows with terminal output.")
        .wrap(true)
        .xalign(0.0)
        .build();
    sb_note.add_css_class("dim-label");
    behavior.attach(&sb_note, 0, 2, 2, 1);

    // Double-click word / triple-click line selection toggle.
    behavior.attach(&row_label("Word/line click select"), 0, 3, 1, 1);
    let word_switch = gtk4::Switch::new();
    word_switch.set_active(state.click_word_select.get());
    word_switch.set_halign(Align::Start);
    word_switch.set_tooltip_text(Some(
        "Double-click selects a word; triple-click selects the whole line.",
    ));
    {
        let state = state.clone();
        word_switch.connect_active_notify(move |sw| {
            state.click_word_select.set(sw.is_active());
            save_config(&state);
        });
    }
    behavior.attach(&word_switch, 1, 3, 1, 1);

    // Auto copy-on-select toggle.
    behavior.attach(&row_label("Copy on select"), 0, 4, 1, 1);
    let copy_switch = gtk4::Switch::new();
    copy_switch.set_active(state.copy_on_select.get());
    copy_switch.set_halign(Align::Start);
    copy_switch.set_tooltip_text(Some(
        "Automatically copy a selection to the clipboard as soon as you make it.",
    ));
    {
        let state = state.clone();
        copy_switch.connect_active_notify(move |sw| {
            state.copy_on_select.set(sw.is_active());
            save_config(&state);
        });
    }
    behavior.attach(&copy_switch, 1, 4, 1, 1);

    // Max tabs spin button.
    behavior.attach(&row_label("Max tabs"), 0, 5, 1, 1);
    let tab_max_spin = SpinButton::with_range(1.0, 100.0, 1.0);
    tab_max_spin.set_value(state.tab_max.get() as f64);
    tab_max_spin.set_halign(Align::Start);
    tab_max_spin.set_tooltip_text(Some("Maximum number of tabs allowed in one window."));
    {
        let state = state.clone();
        tab_max_spin.connect_value_changed(move |s| {
            state.tab_max.set(s.value() as u32);
            save_config(&state);
        });
    }
    behavior.attach(&tab_max_spin, 1, 5, 1, 1);

    // Confirm tab close toggle.
    behavior.attach(&row_label("Confirm tab close"), 0, 6, 1, 1);
    let confirm_tab_switch = gtk4::Switch::new();
    confirm_tab_switch.set_active(state.confirm_tab_close.get());
    confirm_tab_switch.set_halign(Align::Start);
    confirm_tab_switch.set_tooltip_text(Some("Ask for confirmation before closing a tab."));
    {
        let state = state.clone();
        confirm_tab_switch.connect_active_notify(move |sw| {
            state.confirm_tab_close.set(sw.is_active());
            save_config(&state);
        });
    }
    behavior.attach(&confirm_tab_switch, 1, 6, 1, 1);

    // Confirm pane close toggle.
    behavior.attach(&row_label("Confirm pane close"), 0, 7, 1, 1);
    let confirm_pane_switch = gtk4::Switch::new();
    confirm_pane_switch.set_active(state.confirm_pane_close.get());
    confirm_pane_switch.set_halign(Align::Start);
    confirm_pane_switch.set_tooltip_text(Some("Ask for confirmation before closing a pane."));
    {
        let state = state.clone();
        confirm_pane_switch.connect_active_notify(move |sw| {
            state.confirm_pane_close.set(sw.is_active());
            save_config(&state);
        });
    }
    behavior.attach(&confirm_pane_switch, 1, 7, 1, 1);

    // Confirm window close toggle.
    behavior.attach(&row_label("Confirm window close"), 0, 8, 1, 1);
    let confirm_win_switch = gtk4::Switch::new();
    confirm_win_switch.set_active(state.confirm_window_close.get());
    confirm_win_switch.set_halign(Align::Start);
    confirm_win_switch.set_tooltip_text(Some(
        "Ask for confirmation before closing the terminal window.",
    ));
    {
        let state = state.clone();
        confirm_win_switch.connect_active_notify(move |sw| {
            state.confirm_window_close.set(sw.is_active());
            save_config(&state);
        });
    }
    behavior.attach(&confirm_win_switch, 1, 8, 1, 1);

    notebook.append_page(&behavior, Some(&Label::new(Some("Behavior"))));

    // ── Keybindings ──────────────────────────────────────────────────
    let kb_list = ListBox::new();
    kb_list.set_selection_mode(gtk4::SelectionMode::None);
    for (combo, action) in keybinding_reference() {
        let row = gtk4::Box::new(Orientation::Horizontal, 16);
        row.set_margin_start(16);
        row.set_margin_end(16);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        let key = Label::builder()
            .label(combo)
            .xalign(0.0)
            .width_chars(22)
            .build();
        key.add_css_class("monospace");
        let act = Label::builder().label(action).xalign(0.0).hexpand(true).build();
        row.append(&key);
        row.append(&act);
        kb_list.append(&row);
    }
    let kb_scroll = ScrolledWindow::new();
    kb_scroll.set_child(Some(&kb_list));
    kb_scroll.set_hexpand(true);
    kb_scroll.set_vexpand(true);
    notebook.append_page(&kb_scroll, Some(&Label::new(Some("Keybindings"))));

    outer.append(&notebook);

    // Footer
    let footer = gtk4::Box::new(Orientation::Horizontal, 8);
    footer.set_halign(Align::End);
    footer.set_margin_top(8);
    footer.set_margin_bottom(10);
    footer.set_margin_start(10);
    footer.set_margin_end(10);
    let close = Button::with_label("Close");
    {
        let dialog = dialog.clone();
        close.connect_clicked(move |_| dialog.close());
    }
    footer.append(&close);
    outer.append(&footer);

    dialog.set_child(Some(&outer));
    dialog.present();
}

fn keybinding_reference() -> Vec<(&'static str, &'static str)> {
    vec![
        // Splits, tabs, close — the menu accelerators registered via
        // `install_accelerators`. Listed first since these are the chords
        // users picking up from Terminator will reach for.
        ("Ctrl + Shift + O", "Split horizontally (new pane below)"),
        ("Ctrl + Shift + E", "Split vertically (new pane to the right)"),
        ("Ctrl + Shift + T", "Open a new tab"),
        ("Ctrl + A + T", "Open a new tab (alternate shortcut)"),
        ("Ctrl + A + N", "Open a new window"),
        ("Ctrl + Shift + W", "Close the focused pane"),
        // Chord (tmux-style prefix) — alternate keybindings for the same
        // operations plus extras (focus movement, literal Ctrl+A pass-through).
        ("Ctrl + A + ←/↑/↓/→", "Split in that direction"),
        ("Ctrl + A + O", "Cycle focus to the next pane"),
        ("Ctrl + A + h/j/k/l", "Focus pane left / down / up / right"),
        ("Ctrl + A + ' / /", "Previous / next theme (focused pane)"),
        ("Ctrl + A + Ctrl+A", "Send literal Ctrl+A to the shell"),
        // Clipboard.
        ("Ctrl + Shift + C", "Copy selection"),
        ("Ctrl + Shift + V", "Paste from system clipboard"),
        ("Ctrl + Shift + A", "Select all (focused pane)"),
        ("Middle-click", "Paste primary selection"),
        ("Left-drag", "Select text"),
        ("Double-click / triple-click", "Select word / line"),
        // Zoom.
        ("Ctrl + + / =", "Zoom in (focused pane)"),
        ("Ctrl + -", "Zoom out (focused pane)"),
        ("Ctrl + 0", "Reset font size (focused pane)"),
        ("Ctrl + Scroll", "Zoom focused pane"),
        // Misc.
        ("Right-click", "Open the context menu"),
        ("Toolbar ⋯ drag", "Rearrange pane (drop on an edge of another pane)"),
    ]
}

fn paste(pane: &Rc<Pane>, primary: bool) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let clipboard = if primary {
        display.primary_clipboard()
    } else {
        display.clipboard()
    };
    let pane_w = Rc::downgrade(pane);
    clipboard.read_text_async(None::<&gio::Cancellable>, move |result| {
        let Ok(Some(text)) = result else {
            return;
        };
        let Some(p) = pane_w.upgrade() else {
            return;
        };
        let text = text.as_str();
        if text.is_empty() {
            return;
        }
        snap_to_bottom(&p);
        let bracketed = p.parser.borrow().bracketed_paste();
        let mut w = p.writer.borrow_mut();
        if bracketed {
            let _ = w.write_all(b"\x1b[200~");
        }
        if let Err(e) = w.write_all(text.as_bytes()) {
            log::warn!("paste write: {e}");
        }
        if bracketed {
            let _ = w.write_all(b"\x1b[201~");
        }
        let _ = w.flush();
    });
}

fn is_modifier_only(k: gdk::Key) -> bool {
    matches!(
        k,
        gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Hyper_L
            | gdk::Key::Hyper_R
            | gdk::Key::ISO_Level3_Shift
    )
}

fn init_gl_loader() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let lib_names: &[&str] = &["libepoxy.so.0", "libepoxy.0.dylib", "epoxy-0.dll"];
        let library = lib_names
            .iter()
            .find_map(|name| unsafe { libloading::Library::new(name).ok() })
            .expect("libepoxy not found — install gtk4-devel/libepoxy");
        // epoxy needs the symbols available for the program's whole lifetime.
        let library: &'static libloading::Library = Box::leak(Box::new(library));
        epoxy::load_with(|name| unsafe {
            library
                .get::<unsafe extern "C" fn()>(name.as_bytes())
                .map(|sym| *sym as *const _)
                .unwrap_or(std::ptr::null())
        });
    });
}

/// Wrapper that prints PTY bytes as a human-readable string. Used only inside
/// `log::debug!` so it has zero cost when debug is filtered out.
struct DebugBytes<'a>(&'a [u8]);

impl<'a> std::fmt::Debug for DebugBytes<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"")?;
        for &b in self.0 {
            match b {
                0x1b => f.write_str("\\e")?,
                b'\n' => f.write_str("\\n")?,
                b'\r' => f.write_str("\\r")?,
                b'\t' => f.write_str("\\t")?,
                0x20..=0x7e => write!(f, "{}", b as char)?,
                _ => write!(f, "\\x{b:02x}")?,
            }
        }
        f.write_str("\"")
    }
}
