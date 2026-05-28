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
    EventControllerKey, EventControllerScroll, EventControllerScrollFlags, GLArea,
    GestureClick, GestureDrag, Orientation, Paned, PopoverMenu, Scrollbar,
};
use skyterm_core::{
    theme::Theme,
    Grid, Parser,
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
/// focus moves), plus a Fira Code override for the right-click context menu.
/// `font-family` falls back to a generic monospace if Fira Code isn't
/// installed, so this is safe everywhere.
const CSS: &str = "
.pane-wrap {
    border: 2px solid transparent;
}
.pane-wrap.focused {
    border-color: #5eb1ff;
}
popover.menu,
popover.menu label,
popover.menu modelbutton {
    font-family: \"Fira Code\", monospace;
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
        _ => match font::locate_monospace_font() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skyterm: {e}");
                return;
            }
        },
    };
    log::info!("skyterm starting — font {}", font_path.display());

    install_css();

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

    // Build the two menu variants. They share the splits + clipboard
    // sections (gio::Menu sections are referenced, not copied), so we only
    // need to define them once.
    let splits = gio::Menu::new();
    splits.append(Some("Split pane >"), Some("pane.split-right"));
    //splits.append(Some("Split <"), Some("pane.split-left"));
    splits.append(Some("Split pane ^ "), Some("pane.split-up"));
    //splits.append(Some("Split pane ↓"), Some("pane.split-down"));
    splits.append(Some("New tab"), Some("pane.new-tab"));

    let clipboard = gio::Menu::new();
    clipboard.append(Some("Copy"), Some("pane.copy"));
    clipboard.append(Some("Paste"), Some("pane.paste"));
    clipboard.append(Some("Select All"), Some("pane.select-all"));

    let prefs = gio::Menu::new();
    prefs.append(Some("Settings…"), Some("pane.settings"));

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
    window.set_child(Some(&notebook));

    let mut available_themes: Vec<Theme> = Theme::presets();
    available_themes.extend(load_user_themes());

    let initial_theme = cfg
        .theme_name
        .as_deref()
        .and_then(|name| available_themes.iter().find(|t| t.name == name).cloned())
        .unwrap_or_else(Theme::default);

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
    });

    install_pane_actions(&window, &state);

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

/// Register the `pane.split-*` actions on the window. Each one consults
/// `state.menu_target` (set just before the popover is shown) to figure out
/// which pane to split, then dispatches to [`split`].
fn install_pane_actions(window: &ApplicationWindow, state: &Rc<WindowState>) {
    let group = gio::SimpleActionGroup::new();
    for (name, dir) in [
        ("split-right", SplitDir::Right),
        ("split-left", SplitDir::Left),
        ("split-up", SplitDir::Up),
        ("split-down", SplitDir::Down),
    ] {
        let action = gio::SimpleAction::new(name, None);
        let state = state.clone();
        action.connect_activate(move |_, _| {
            let target = state.menu_target.borrow().clone();
            if let Some(target) = target {
                split(&state, &target, dir);
            }
        });
        group.add_action(&action);
    }

    let close = gio::SimpleAction::new("close", None);
    {
        let state = state.clone();
        close.connect_activate(move |_, _| {
            let target = state.menu_target.borrow().clone();
            if let Some(target) = target {
                close_pane(&state, &target);
            }
        });
    }
    group.add_action(&close);

    let copy = gio::SimpleAction::new("copy", None);
    {
        let state = state.clone();
        copy.connect_activate(move |_, _| {
            if let Some(target) = state.menu_target.borrow().clone() {
                copy_selection(&target);
            }
        });
    }
    group.add_action(&copy);

    let paste_action = gio::SimpleAction::new("paste", None);
    {
        let state = state.clone();
        paste_action.connect_activate(move |_, _| {
            if let Some(target) = state.menu_target.borrow().clone() {
                paste(&target, false);
            }
        });
    }
    group.add_action(&paste_action);

    let select_all = gio::SimpleAction::new("select-all", None);
    {
        let state = state.clone();
        select_all.connect_activate(move |_, _| {
            if let Some(target) = state.menu_target.borrow().clone() {
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

    let settings_action = gio::SimpleAction::new("settings", None);
    {
        let state = state.clone();
        settings_action.connect_activate(move |_, _| {
            open_settings(&state);
        });
    }
    group.add_action(&settings_action);

    window.insert_action_group("pane", Some(&group));
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

/// Spawn a new tab with one fresh pane and make it active. Returns false if
/// PTY / atlas / GLArea setup failed.
fn new_tab(state: &Rc<WindowState>) -> bool {
    let tab = match make_tab(state.clone(), INITIAL_COLS, INITIAL_ROWS) {
        Some(t) => t,
        None => return false,
    };
    let page = state
        .notebook
        .append_page(&tab.container, Some(&tab.tab_label));
    state.tabs.borrow_mut().push(tab.clone());
    state.notebook.set_show_tabs(state.tabs.borrow().len() > 1);
    state.notebook.set_current_page(Some(page));
    if let Some(p) = tab.panes.borrow().first().cloned() {
        focus_pane(state, &p);
    }
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
    let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    close_btn.set_has_frame(false);
    close_btn.add_css_class("flat");
    let tab_label = gtk4::Box::new(Orientation::Horizontal, 6);
    tab_label.append(&title_label);
    tab_label.append(&close_btn);

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
            if let Some(t) = tab_w.upgrade() {
                close_tab(&state, &t);
            }
        });
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

fn install_css() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let provider = CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
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

    let inner = gtk4::Box::new(Orientation::Horizontal, 0);
    inner.append(&gl_area);
    inner.append(&scrollbar);

    let wrap = gtk4::Box::new(Orientation::Vertical, 0);
    wrap.append(&inner);
    wrap.add_css_class("pane-wrap");
    wrap.set_hexpand(true);
    wrap.set_vexpand(true);

    let pane = Rc::new(Pane {
        wrap,
        gl_area,
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
        theme: state.theme.clone(),
        selection: RefCell::new(None),
        _child: RefCell::new(pty_handle.child),
    });

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
            let atlas = match font::build_atlas(&path, size) {
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
            let w = area.width();
            let h = area.height();
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
        pane.gl_area.connect_render(move |area, _ctx| {
            if let Some(p) = pane_w.upgrade() {
                if let Some(r) = p.renderer.borrow_mut().as_mut() {
                    let w = area.width();
                    let h = area.height();
                    if w > 0 && h > 0 {
                        r.resize(w, h);
                    }
                    let sel = *p.selection.borrow();
                    r.render(&p.grid.borrow(), sel, &p.theme.borrow());
                }
            }
            glib::Propagation::Stop
        });
    }

    // PTY read loop — feeds bytes into this pane's parser, returns DSR/DA
    // responses, kicks a redraw. Holds only a Weak<Pane> so the loop exits
    // once the pane is dropped.
    {
        let pane_w = Rc::downgrade(&pane);
        glib::spawn_future_local(async move {
            while let Ok(bytes) = rx.recv().await {
                let Some(p) = pane_w.upgrade() else {
                    break;
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
        });
    }

    Some(pane)
}

/// Hook a pane up to keyboard, mouse-wheel scroll, click-to-focus, middle-
/// click paste, and widget resize. Done as a separate step from `make_pane`
/// because handlers need an `Rc<WindowState>` clone.
fn wire_pane(state: &Rc<WindowState>, pane: &Rc<Pane>) {
    // Resize → reflow grid + PTY.
    {
        let pane_w = Rc::downgrade(pane);
        pane.gl_area.connect_resize(move |_area, w, h| {
            let Some(p) = pane_w.upgrade() else {
                return;
            };
            if let Some(r) = p.renderer.borrow_mut().as_mut() {
                r.resize(w, h);
            }
            if w <= 0 || h <= 0 {
                return;
            }
            let (cw, ch) = p.cell_dims.get();
            let new_cols = (((w as u32) / cw).max(1)) as u16;
            let new_rows = (((h as u32) / ch).max(1)) as u16;
            let (cur_cols, cur_rows) = {
                let g = p.grid.borrow();
                (g.cols() as u16, g.rows() as u16)
            };
            if new_cols != cur_cols || new_rows != cur_rows {
                p.grid
                    .borrow_mut()
                    .resize(new_cols as usize, new_rows as usize);
                let r = p.master.borrow_mut().resize(portable_pty::PtySize {
                    cols: new_cols,
                    rows: new_rows,
                    pixel_width: w as u16,
                    pixel_height: h as u16,
                });
                if let Err(e) = r {
                    log::warn!("pty resize: {e}");
                }
            }
            sync_scrollbar(&p);
            p.gl_area.queue_render();
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

    // Mouse wheel — Ctrl+wheel zooms the *focused* pane (which may not be the
    // hovered one); plain wheel scrolls this pane's scrollback.
    {
        let state = state.clone();
        let pane_w = Rc::downgrade(pane);
        let scroll_ctrl = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll_ctrl.connect_scroll(move |ctrl, _dx, dy| {
            let mods = ctrl.current_event_state();
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
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
            // Scrolling changes which cells the view shows, so any anchored
            // selection would silently start covering different content.
            // Clearer to drop it.
            clear_selection(&p);
            let lines_per_notch = 3.0;
            let new_value = (p.scroll_adj.value() + dy * lines_per_notch)
                .max(p.scroll_adj.lower())
                .min((p.scroll_adj.upper() - p.scroll_adj.page_size()).max(0.0));
            p.scroll_adj.set_value(new_value);
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
        {
            let state = state.clone();
            let pane_w = pane_w.clone();
            drag.connect_begin(move |_, _| {
                if let Some(p) = pane_w.upgrade() {
                    focus_pane(&state, &p);
                    clear_selection(&p);
                }
            });
        }
        {
            let pane_w = pane_w.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let Some(p) = pane_w.upgrade() else { return };
                if let Some(cell) = pixel_to_cell(&p, x, y) {
                    *p.selection.borrow_mut() = Some(Selection {
                        anchor: cell,
                        active: cell,
                    });
                    p.gl_area.queue_render();
                }
            });
        }
        {
            let pane_w = pane_w.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                let Some(p) = pane_w.upgrade() else { return };
                let Some((sx, sy)) = g.start_point() else { return };
                if let Some(cell) = pixel_to_cell(&p, sx + dx, sy + dy) {
                    if let Some(sel) = p.selection.borrow_mut().as_mut() {
                        sel.active = cell;
                    }
                    p.gl_area.queue_render();
                }
            });
        }
        pane.gl_area.add_controller(drag);
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
    // PopoverMenu wraps its content in an internal GtkScrolledWindow whose
    // default max-content-height is small enough that our 6-item menu
    // overflows and gets a scrollbar. Override on every show — the inner
    // widgets are created lazily so we can't catch them at construction.
    popover.connect_show(|p| {
        unconstrain_scrolled_windows(p.upcast_ref::<gtk4::Widget>());
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

            // 2) Prefix: Ctrl+A — arm chord, swallow.
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(keyval, gdk::Key::a | gdk::Key::A)
            {
                state.chord_at.set(Some(Instant::now()));
                return glib::Propagation::Stop;
            }

            // 3) Ctrl+Shift+V — paste from system clipboard.
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                && matches!(keyval, gdk::Key::v | gdk::Key::V)
            {
                paste(&p, false);
                return glib::Propagation::Stop;
            }

            // 4) Ctrl+± font zoom on the focused pane.
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

            // 5) Default: encode the keystroke and write to the PTY. Typing
            // input also clears the selection — the user has clearly moved
            // on from "I'm picking text to copy" — and snaps the view back
            // to the live screen if they were scrolled up.
            let bytes = input::encode_key(keyval, modifiers);
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
    }
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

    // The new pane lives in the same tab as the focused one. (`tab_of_pane`
    // is cheap — a linear scan over a small tabs vec — and avoids threading
    // the tab through every callsite.)
    if let Some(tab) = tab_of_pane(state, focused) {
        tab.panes.borrow_mut().push(new_pane.clone());
    }
    wire_pane(state, &new_pane);
    focus_pane(state, &new_pane);
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

        let mut lines: Vec<String> = Vec::with_capacity(er - sr + 1);
        for r in sr..=er {
            let from = if r == sr { sc } else { 0 };
            let to = if r == er { ec } else { cols - 1 };
            let mut line = String::with_capacity(to - from + 1);
            for c in from..=to {
                line.push(g.visible_cell(r, c).ch);
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
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
fn unconstrain_scrolled_windows(widget: &gtk4::Widget) {
    if let Some(sw) = widget.downcast_ref::<gtk4::ScrolledWindow>() {
        sw.set_propagate_natural_height(true);
        sw.set_propagate_natural_width(true);
        sw.set_max_content_height(-1);
        sw.set_vscrollbar_policy(gtk4::PolicyType::Never);
        sw.set_hscrollbar_policy(gtk4::PolicyType::Never);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        unconstrain_scrolled_windows(&c);
        child = c.next_sibling();
    }
}

fn pixel_to_cell(pane: &Pane, x: f64, y: f64) -> Option<(usize, usize)> {
    let (cw, ch) = pane.cell_dims.get();
    if cw == 0 || ch == 0 {
        return None;
    }
    let col = (x.max(0.0) as u32 / cw) as usize;
    let row = (y.max(0.0) as u32 / ch) as usize;
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
    let new_atlas = match font::build_atlas(&path, target) {
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

/// Walk `~/.config/skyterm/themes/` and parse every file inside as a
/// Terminator-style theme config. Files with no parseable themes are
/// silently skipped (a non-theme stray file shouldn't break startup).
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
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.extend(skyterm_core::theme::parse_terminator_themes(&text));
    }
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
    // Pane.font_path is a clone of the same Rc, so it already sees the new
    // value. We just need each pane to re-rasterize its atlas at the
    // current size.
    for tab in state.tabs.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            let size = pane.font_size.get();
            let Ok(atlas) = font::build_atlas(&new_path, size) else {
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

/// Swap the active color theme. `Cell` colors are palette references, so
/// existing text re-tints on the next render.
fn apply_theme(state: &Rc<WindowState>, theme: Theme) {
    *state.theme.borrow_mut() = theme;
    for tab in state.tabs.borrow().iter() {
        for pane in tab.panes.borrow().iter() {
            pane.gl_area.queue_render();
        }
    }
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
    };
    if let Err(e) = cfg.save(&path) {
        log::warn!("save config: {e}");
    }
}

/// Open the Settings modal — Appearance / Theme / Behavior / Keybindings.
/// Every control applies live and writes back to the config file.
fn open_settings(state: &Rc<WindowState>) {
    use gtk4::{Align, Button, DropDown, Grid as GtkGrid, Label, ListBox, ScrolledWindow,
        SpinButton, StringList};

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

    theme_page.attach(&row_label("Color theme"), 0, 0, 1, 1);
    let themes = state.available_themes.clone();
    let theme_names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
    let theme_model = StringList::new(&theme_names);
    let theme_dd = DropDown::new(Some(theme_model), gtk4::Expression::NONE);
    theme_dd.set_hexpand(true);
    let current_theme_name = state.theme.borrow().name.clone();
    if let Some(idx) = themes.iter().position(|t| t.name == current_theme_name) {
        theme_dd.set_selected(idx as u32);
    }
    {
        let state = state.clone();
        let themes = themes.clone();
        theme_dd.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            if let Some(t) = themes.get(idx) {
                apply_theme(&state, t.clone());
                save_config(&state);
            }
        });
    }
    theme_page.attach(&theme_dd, 1, 0, 1, 1);

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

    behavior.attach(&row_label("Scrollback lines"), 0, 0, 1, 1);
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
    behavior.attach(&scrollback_spin, 1, 0, 1, 1);

    let sb_note = Label::builder()
        .label("0 = infinite (no cap). Memory grows with terminal output.")
        .wrap(true)
        .xalign(0.0)
        .build();
    sb_note.add_css_class("dim-label");
    behavior.attach(&sb_note, 0, 1, 2, 1);

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
        ("Ctrl+A → →", "Split pane to the right"),
        ("Ctrl+A → ↑", "Split pane above"),
        ("Ctrl+A → t", "Open a new tab"),
        ("Ctrl+A → o", "Cycle focus to the next pane"),
        ("Ctrl+A → h/j/k/l", "Focus pane left / down / up / right"),
        ("Ctrl+A → Ctrl+A", "Send literal Ctrl+A to the shell"),
        ("Ctrl + + / =", "Zoom in (focused pane)"),
        ("Ctrl + -", "Zoom out (focused pane)"),
        ("Ctrl + 0", "Reset font size (focused pane)"),
        ("Ctrl + Scroll", "Zoom focused pane"),
        ("Ctrl+Shift+V", "Paste from system clipboard"),
        ("Middle-click", "Paste primary selection"),
        ("Left-drag", "Select text"),
        ("Right-click", "Open the context menu"),
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
