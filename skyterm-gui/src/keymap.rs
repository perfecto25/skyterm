//! Keyboard action registry.
//!
//! One place answers three questions that used to be answered in three
//! different places (the chord handler, `install_accelerators`, and the
//! Settings list): what actions exist, which key combinations run them, and
//! how those combinations round-trip through `config.toml`.
//!
//! An [`Action`]'s [`id`](Action::id) is also its `pane.*` GAction name, so a
//! non-chord binding becomes an application accelerator and GTK renders it
//! next to the matching right-click menu item for free.
//!
//! Two built-in sets ship: [`ShortcutStyle::Skyterm`] (the default — a
//! tmux-style `Ctrl+A` prefix for pane/tab management) and
//! [`ShortcutStyle::Terminator`] (`Ctrl+Shift+…` chords, no prefix, for people
//! migrating). [`ShortcutStyle::Custom`] is whatever the user recorded in
//! Settings, stored per-action in the config file.

use std::collections::{BTreeMap, HashMap};

use gtk4::gdk;

/// Every keyboard-triggerable operation. The variant order is the order rows
/// appear in Settings ▸ Keybindings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    SplitDown,
    SplitRight,
    SplitUp,
    SplitLeft,
    FocusPane,
    ClosePane,
    NewTab,
    NewWindow,
    MaximizeWindow,
    MinimizeWindow,
    NextTab,
    PrevTab,
    FocusNext,
    FocusLeft,
    FocusDown,
    FocusUp,
    FocusRight,
    Copy,
    Paste,
    SelectAll,
    ShowMenu,
    ShowShortcuts,
    ThemePrev,
    ThemeNext,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    SendPrefix,
}

impl Action {
    /// Display order for Settings and the registration loop.
    pub const fn all() -> &'static [Action] {
        use Action::*;
        &[
            SplitDown, SplitRight, SplitUp, SplitLeft, FocusPane, ClosePane,
            NewTab, NewWindow, MaximizeWindow, MinimizeWindow, NextTab, PrevTab,
            FocusNext, FocusLeft, FocusDown, FocusUp, FocusRight,
            Copy, Paste, SelectAll,
            ShowMenu, ShowShortcuts,
            ThemePrev, ThemeNext,
            ZoomIn, ZoomOut, ZoomReset,
            SendPrefix,
        ]
    }

    /// The `pane.*` GAction name (without the `pane.` prefix) and the key used
    /// for this action in the config file's `[keybindings]` table.
    ///
    /// `split-horizontal` / `split-vertical` keep their historical names —
    /// they're referenced by the right-click menu model, where "horizontal"
    /// means a horizontal divider (new pane *below*).
    pub const fn id(self) -> &'static str {
        match self {
            Action::SplitDown => "split-horizontal",
            Action::SplitRight => "split-vertical",
            Action::SplitUp => "split-up",
            Action::SplitLeft => "split-left",
            Action::FocusPane => "focus-pane",
            Action::ClosePane => "close",
            Action::NewTab => "new-tab",
            Action::NewWindow => "new-window",
            Action::MaximizeWindow => "maximize",
            Action::MinimizeWindow => "minimize",
            Action::NextTab => "next-tab",
            Action::PrevTab => "prev-tab",
            Action::FocusNext => "focus-next",
            Action::FocusLeft => "focus-left",
            Action::FocusDown => "focus-down",
            Action::FocusUp => "focus-up",
            Action::FocusRight => "focus-right",
            Action::Copy => "copy",
            Action::Paste => "paste",
            Action::SelectAll => "select-all",
            Action::ShowMenu => "show-menu",
            Action::ShowShortcuts => "show-shortcuts",
            Action::ThemePrev => "theme-prev",
            Action::ThemeNext => "theme-next",
            Action::ZoomIn => "zoom-in",
            Action::ZoomOut => "zoom-out",
            Action::ZoomReset => "zoom-reset",
            Action::SendPrefix => "send-prefix",
        }
    }

    /// Human-readable description shown in Settings ▸ Keybindings.
    pub const fn label(self) -> &'static str {
        match self {
            Action::SplitDown => "Split pane down (new pane below)",
            Action::SplitRight => "Split pane right (new pane to the right)",
            Action::SplitUp => "Split pane up (new pane above)",
            Action::SplitLeft => "Split pane left (new pane to the left)",
            Action::FocusPane => "Focus pane — enlarge the active pane, press again to restore",
            Action::ClosePane => "Close the focused pane",
            Action::NewTab => "Open a new tab",
            Action::NewWindow => "Open a new window",
            Action::MaximizeWindow => "Maximize the terminal window (press again to restore)",
            Action::MinimizeWindow => "Minimize the terminal window",
            Action::NextTab => "Next tab",
            Action::PrevTab => "Previous tab",
            Action::FocusNext => "Cycle focus to the next pane",
            Action::FocusLeft => "Focus the pane to the left",
            Action::FocusDown => "Focus the pane below",
            Action::FocusUp => "Focus the pane above",
            Action::FocusRight => "Focus the pane to the right",
            Action::Copy => "Copy selection",
            Action::Paste => "Paste from system clipboard",
            Action::SelectAll => "Select all (focused pane)",
            Action::ShowMenu => "Open the context menu (same as right-click)",
            Action::ShowShortcuts => "Show this shortcut reference",
            Action::ThemePrev => "Previous theme (focused pane)",
            Action::ThemeNext => "Next theme (focused pane)",
            Action::ZoomIn => "Zoom in (focused pane)",
            Action::ZoomOut => "Zoom out (focused pane)",
            Action::ZoomReset => "Reset font size (focused pane)",
            Action::SendPrefix => "Send a literal Ctrl+A to the shell",
        }
    }

    fn from_id(id: &str) -> Option<Action> {
        Action::all().iter().copied().find(|a| a.id() == id)
    }
}

/// Which set of bindings is active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShortcutStyle {
    /// skyterm's own bindings — the default.
    Skyterm,
    /// Terminator-compatible bindings for people migrating.
    Terminator,
    /// Per-action bindings recorded by the user in Settings.
    Custom,
}

impl ShortcutStyle {
    /// Dropdown order in Settings. Skyterm is first, and therefore default.
    pub const ALL: &'static [ShortcutStyle] = &[
        ShortcutStyle::Skyterm,
        ShortcutStyle::Terminator,
        ShortcutStyle::Custom,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            ShortcutStyle::Skyterm => "skyterm",
            ShortcutStyle::Terminator => "terminator",
            ShortcutStyle::Custom => "custom",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            ShortcutStyle::Skyterm => "Skyterm",
            ShortcutStyle::Terminator => "Terminator",
            ShortcutStyle::Custom => "Custom",
        }
    }

    /// Unknown / missing ids fall back to `Skyterm`.
    pub fn from_id(s: &str) -> ShortcutStyle {
        ShortcutStyle::ALL
            .iter()
            .copied()
            .find(|st| st.id() == s)
            .unwrap_or(ShortcutStyle::Skyterm)
    }
}

/// One key combination. `prefix` means "only after the `Ctrl+A` chord prefix";
/// such bindings are matched by hand in the key controller. Everything else is
/// handed to GTK as an application accelerator.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Binding {
    pub prefix: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Spec name from [`KEYS`] — always lowercase, e.g. `down`, `x`, `slash`.
    pub key: String,
}

impl Binding {
    /// Parse the config / default-table form: `+`-separated modifier tokens
    /// followed by the key name, e.g. `prefix+down`, `ctrl+shift+o`,
    /// `prefix+ctrl+a`. The key name is always the last token, so `ctrl+plus`
    /// is unambiguous.
    pub fn parse(spec: &str) -> Option<Binding> {
        let spec = spec.trim().to_ascii_lowercase();
        if spec.is_empty() {
            return None;
        }
        let mut parts: Vec<&str> = spec.split('+').filter(|p| !p.is_empty()).collect();
        let key = parts.pop()?.to_string();
        if key_info(&key).is_none() {
            log::warn!("keybinding: unknown key '{key}' in '{spec}'");
            return None;
        }
        let mut b = Binding { prefix: false, ctrl: false, shift: false, alt: false, key };
        for p in parts {
            match p {
                "prefix" => b.prefix = true,
                "ctrl" | "primary" | "control" => b.ctrl = true,
                "shift" => b.shift = true,
                "alt" | "meta" => b.alt = true,
                other => {
                    log::warn!("keybinding: unknown modifier '{other}' in '{spec}'");
                    return None;
                }
            }
        }
        Some(b)
    }

    /// Serialize back to the `parse` form (what lands in `config.toml`).
    pub fn spec(&self) -> String {
        let mut s = String::new();
        if self.prefix {
            s.push_str("prefix+");
        }
        if self.ctrl {
            s.push_str("ctrl+");
        }
        if self.shift {
            s.push_str("shift+");
        }
        if self.alt {
            s.push_str("alt+");
        }
        s.push_str(&self.key);
        s
    }

    /// Pretty form for Settings, e.g. `Ctrl + A + Down`, `Ctrl + Shift + C`.
    pub fn display(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.prefix {
            parts.push(primary_label().to_string());
            parts.push("A".to_string());
        }
        if self.ctrl {
            parts.push(primary_label().to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push(if cfg!(target_os = "macos") { "⌥".to_string() } else { "Alt".to_string() });
        }
        parts.push(
            key_info(&self.key)
                .map(|k| k.display.to_string())
                .unwrap_or_else(|| self.key.clone()),
        );
        parts.join(" + ")
    }

    /// GTK accelerator string, or `None` for prefix-chord bindings (GTK has no
    /// notion of our two-key chord — those are matched in the key controller).
    /// `<Primary>` resolves to Ctrl on Linux/Windows and ⌘ on macOS.
    pub fn accel(&self) -> Option<String> {
        if self.prefix {
            return None;
        }
        let info = key_info(&self.key)?;
        let mut s = String::new();
        if self.ctrl {
            s.push_str("<Primary>");
        }
        if self.shift {
            s.push_str("<Shift>");
        }
        if self.alt {
            s.push_str("<Alt>");
        }
        s.push_str(info.gdk_name);
        Some(s)
    }

    /// True when this binding can safely be sent to GTK / typed without
    /// stealing an ordinary keystroke from the shell. Bare letters would make
    /// the terminal unusable, so Settings refuses to record them.
    pub fn is_safe(&self) -> bool {
        // A function key is unambiguous on its own; anything else needs the
        // chord prefix or a Ctrl / Alt modifier. Shift alone doesn't count —
        // Shift+letter is just a capital letter.
        let f_key = self.key.len() > 1
            && self.key.starts_with('f')
            && self.key[1..].bytes().all(|b| b.is_ascii_digit());
        self.prefix || self.ctrl || self.alt || f_key
    }
}

/// `Ctrl` on Linux/Windows, `⌘` on macOS — matches the modifier GTK's
/// `<Primary>` resolves to and the chord prefix used in the key controller.
fn primary_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

/// The bindings for one style, as `(action, specs)`. Actions absent from a
/// table are unbound in that style.
type Defaults = &'static [(Action, &'static [&'static str])];

/// skyterm's own set: a tmux-style `Ctrl+A` prefix for anything that
/// rearranges panes / tabs, plus the terminal-universal `Ctrl+Shift+C/V/A`
/// and `Ctrl+±` that have no sane prefix-based alternative.
const SKYTERM_DEFAULTS: Defaults = &[
    (Action::SplitDown, &["prefix+down"]),
    (Action::SplitRight, &["prefix+right"]),
    (Action::SplitUp, &["prefix+up"]),
    (Action::SplitLeft, &["prefix+left"]),
    (Action::FocusPane, &["prefix+f"]),
    (Action::ClosePane, &["prefix+x"]),
    (Action::NewTab, &["prefix+t"]),
    (Action::NewWindow, &["ctrl+shift+n"]),
    // ] grows the window, [ tucks it away — the brackets point the way.
    (Action::MaximizeWindow, &["prefix+bracketright"]),
    (Action::MinimizeWindow, &["prefix+bracketleft"]),
    (Action::NextTab, &["prefix+tab"]),
    (Action::PrevTab, &["prefix+shift+tab"]),
    (Action::FocusNext, &["prefix+o"]),
    (Action::FocusLeft, &["prefix+h"]),
    (Action::FocusDown, &["prefix+j"]),
    (Action::FocusUp, &["prefix+k"]),
    (Action::FocusRight, &["prefix+l"]),
    (Action::Copy, &["prefix+c"]),
    (Action::Paste, &["prefix+v"]),
    // `prefix+a` (no Ctrl) vs `prefix+ctrl+a` (SendPrefix) — see the two-pass
    // Ctrl matching in `chord_action`, which is what keeps these apart.
    (Action::SelectAll, &["prefix+a"]),
    (Action::ShowMenu, &["prefix+return"]),
    (Action::ShowShortcuts, &["prefix+backspace"]),
    (Action::ThemePrev, &["prefix+apostrophe"]),
    (Action::ThemeNext, &["prefix+slash"]),
    (Action::ZoomIn, &["ctrl+plus", "ctrl+equal"]),
    (Action::ZoomOut, &["ctrl+minus"]),
    (Action::ZoomReset, &["ctrl+0"]),
    (Action::SendPrefix, &["prefix+ctrl+a"]),
];

/// Terminator-compatible set. No chord prefix at all, so `Ctrl+A` reaches the
/// shell as readline's beginning-of-line. Theme cycling has no Terminator
/// equivalent, so it gets the otherwise-unused bracket chords.
const TERMINATOR_DEFAULTS: Defaults = &[
    (Action::SplitDown, &["ctrl+shift+o"]),
    (Action::SplitRight, &["ctrl+shift+e"]),
    // Terminator's own "toggle maximize terminal".
    (Action::FocusPane, &["ctrl+shift+x"]),
    (Action::ClosePane, &["ctrl+shift+w"]),
    (Action::NewTab, &["ctrl+shift+t"]),
    (Action::NewWindow, &["ctrl+shift+i"]),
    // Terminator's only window-size key is F11 (its "full screen" toggle).
    // There's no Terminator binding for minimize, so it stays unbound here.
    (Action::MaximizeWindow, &["f11"]),
    (Action::NextTab, &["ctrl+pagedown"]),
    (Action::PrevTab, &["ctrl+pageup"]),
    (Action::FocusNext, &["ctrl+tab"]),
    (Action::FocusLeft, &["alt+left"]),
    (Action::FocusDown, &["alt+down"]),
    (Action::FocusUp, &["alt+up"]),
    (Action::FocusRight, &["alt+right"]),
    (Action::Copy, &["ctrl+shift+c"]),
    (Action::Paste, &["ctrl+shift+v"]),
    (Action::SelectAll, &["ctrl+shift+a"]),
    (Action::ThemePrev, &["ctrl+shift+bracketleft"]),
    (Action::ThemeNext, &["ctrl+shift+bracketright"]),
    (Action::ZoomIn, &["ctrl+plus", "ctrl+equal"]),
    (Action::ZoomOut, &["ctrl+minus"]),
    (Action::ZoomReset, &["ctrl+0"]),
];

/// The active bindings, resolved from a style plus (for `Custom`) the user's
/// recorded overrides.
pub struct Keymap {
    style: ShortcutStyle,
    map: HashMap<Action, Vec<Binding>>,
}

impl Keymap {
    /// Build the map for `style`. `custom` is the config's per-action override
    /// table (action id → comma-separated specs, empty value = unbound); it is
    /// only consulted for [`ShortcutStyle::Custom`], where it layers on top of
    /// the skyterm defaults so a partially-filled table still works.
    pub fn new(style: ShortcutStyle, custom: &BTreeMap<String, String>) -> Keymap {
        let table = match style {
            ShortcutStyle::Terminator => TERMINATOR_DEFAULTS,
            _ => SKYTERM_DEFAULTS,
        };
        let mut map: HashMap<Action, Vec<Binding>> = HashMap::new();
        for (action, specs) in table {
            map.insert(*action, specs.iter().filter_map(|s| Binding::parse(s)).collect());
        }
        if style == ShortcutStyle::Custom {
            for (id, specs) in custom {
                let Some(action) = Action::from_id(id) else {
                    log::warn!("keybinding: unknown action '{id}' in config");
                    continue;
                };
                map.insert(
                    action,
                    specs
                        .split(',')
                        .filter(|s| !s.trim().is_empty())
                        .filter_map(Binding::parse)
                        .collect(),
                );
            }
        }
        Keymap { style, map }
    }

    pub fn style(&self) -> ShortcutStyle {
        self.style
    }

    pub fn bindings(&self, action: Action) -> &[Binding] {
        self.map.get(&action).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// GTK accelerator strings for `action` — prefix-chord bindings excluded.
    pub fn accels(&self, action: Action) -> Vec<String> {
        self.bindings(action).iter().filter_map(|b| b.accel()).collect()
    }

    /// Whether the `Ctrl+A` chord prefix should be armed at all. False for a
    /// style with no prefix bindings, so `Ctrl+A` passes through to the shell.
    pub fn prefix_enabled(&self) -> bool {
        self.map.values().flatten().any(|b| b.prefix)
    }

    /// Resolve the second key of an armed chord.
    ///
    /// `shift` / `alt` must always match exactly. `Ctrl` is matched in two
    /// passes: an exact one first, then one that ignores a held Ctrl. The
    /// second pass exists because the user has just pressed `Ctrl+A` and
    /// usually keeps Ctrl down for the second key, so `Ctrl+A` → `Ctrl+X` has
    /// to reach the `prefix+x` binding. The exact pass has to come first, or a
    /// loose binding would swallow a deliberately Ctrl-qualified one — that's
    /// exactly the `prefix+a` (select all) vs `prefix+ctrl+a` (send a literal
    /// Ctrl+A) pair, which is only distinguishable by Ctrl.
    pub fn chord_action(
        &self,
        modifiers: gdk::ModifierType,
        keyval: gdk::Key,
    ) -> Option<Action> {
        let spec = keyval_spec(keyval)?;
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        // Search in `Action::all()` order within each pass, so the result is
        // deterministic regardless of HashMap iteration order.
        let matches = |b: &Binding, exact_ctrl: bool| {
            b.prefix
                && b.key == spec
                && b.shift == shift
                && b.alt == alt
                && if exact_ctrl { b.ctrl == ctrl } else { !b.ctrl || ctrl }
        };
        for exact_ctrl in [true, false] {
            let hit = Action::all()
                .iter()
                .copied()
                .find(|a| self.bindings(*a).iter().any(|b| matches(b, exact_ctrl)));
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// Flatten to the config table form. Used when the user switches to
    /// `Custom`: the set they were just using becomes their starting point.
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        Action::all()
            .iter()
            .map(|a| {
                let specs = self
                    .bindings(*a)
                    .iter()
                    .map(|b| b.spec())
                    .collect::<Vec<_>>()
                    .join(",");
                (a.id().to_string(), specs)
            })
            .collect()
    }
}

/// One bindable key: how it's written in config (`spec`), the GDK key name GTK
/// wants inside an accelerator string (`gdk_name`), and how Settings shows it.
pub struct KeyInfo {
    pub spec: &'static str,
    pub gdk_name: &'static str,
    pub display: &'static str,
}

/// Every key skyterm can bind. Kept in sync by hand with [`keyval_spec`] —
/// that function is the runtime matcher, this table is the config/display side.
pub const KEYS: &[KeyInfo] = &[
    KeyInfo { spec: "a", gdk_name: "a", display: "A" },
    KeyInfo { spec: "b", gdk_name: "b", display: "B" },
    KeyInfo { spec: "c", gdk_name: "c", display: "C" },
    KeyInfo { spec: "d", gdk_name: "d", display: "D" },
    KeyInfo { spec: "e", gdk_name: "e", display: "E" },
    KeyInfo { spec: "f", gdk_name: "f", display: "F" },
    KeyInfo { spec: "g", gdk_name: "g", display: "G" },
    KeyInfo { spec: "h", gdk_name: "h", display: "H" },
    KeyInfo { spec: "i", gdk_name: "i", display: "I" },
    KeyInfo { spec: "j", gdk_name: "j", display: "J" },
    KeyInfo { spec: "k", gdk_name: "k", display: "K" },
    KeyInfo { spec: "l", gdk_name: "l", display: "L" },
    KeyInfo { spec: "m", gdk_name: "m", display: "M" },
    KeyInfo { spec: "n", gdk_name: "n", display: "N" },
    KeyInfo { spec: "o", gdk_name: "o", display: "O" },
    KeyInfo { spec: "p", gdk_name: "p", display: "P" },
    KeyInfo { spec: "q", gdk_name: "q", display: "Q" },
    KeyInfo { spec: "r", gdk_name: "r", display: "R" },
    KeyInfo { spec: "s", gdk_name: "s", display: "S" },
    KeyInfo { spec: "t", gdk_name: "t", display: "T" },
    KeyInfo { spec: "u", gdk_name: "u", display: "U" },
    KeyInfo { spec: "v", gdk_name: "v", display: "V" },
    KeyInfo { spec: "w", gdk_name: "w", display: "W" },
    KeyInfo { spec: "x", gdk_name: "x", display: "X" },
    KeyInfo { spec: "y", gdk_name: "y", display: "Y" },
    KeyInfo { spec: "z", gdk_name: "z", display: "Z" },
    KeyInfo { spec: "0", gdk_name: "0", display: "0" },
    KeyInfo { spec: "1", gdk_name: "1", display: "1" },
    KeyInfo { spec: "2", gdk_name: "2", display: "2" },
    KeyInfo { spec: "3", gdk_name: "3", display: "3" },
    KeyInfo { spec: "4", gdk_name: "4", display: "4" },
    KeyInfo { spec: "5", gdk_name: "5", display: "5" },
    KeyInfo { spec: "6", gdk_name: "6", display: "6" },
    KeyInfo { spec: "7", gdk_name: "7", display: "7" },
    KeyInfo { spec: "8", gdk_name: "8", display: "8" },
    KeyInfo { spec: "9", gdk_name: "9", display: "9" },
    KeyInfo { spec: "up", gdk_name: "Up", display: "Up" },
    KeyInfo { spec: "down", gdk_name: "Down", display: "Down" },
    KeyInfo { spec: "left", gdk_name: "Left", display: "Left" },
    KeyInfo { spec: "right", gdk_name: "Right", display: "Right" },
    KeyInfo { spec: "tab", gdk_name: "Tab", display: "Tab" },
    KeyInfo { spec: "pageup", gdk_name: "Page_Up", display: "Page Up" },
    KeyInfo { spec: "pagedown", gdk_name: "Page_Down", display: "Page Down" },
    KeyInfo { spec: "home", gdk_name: "Home", display: "Home" },
    KeyInfo { spec: "end", gdk_name: "End", display: "End" },
    KeyInfo { spec: "insert", gdk_name: "Insert", display: "Insert" },
    KeyInfo { spec: "delete", gdk_name: "Delete", display: "Delete" },
    KeyInfo { spec: "space", gdk_name: "space", display: "Space" },
    KeyInfo { spec: "return", gdk_name: "Return", display: "Enter" },
    KeyInfo { spec: "backspace", gdk_name: "BackSpace", display: "Backspace" },
    KeyInfo { spec: "escape", gdk_name: "Escape", display: "Esc" },
    KeyInfo { spec: "apostrophe", gdk_name: "apostrophe", display: "'" },
    KeyInfo { spec: "slash", gdk_name: "slash", display: "/" },
    KeyInfo { spec: "backslash", gdk_name: "backslash", display: "\\" },
    KeyInfo { spec: "grave", gdk_name: "grave", display: "`" },
    KeyInfo { spec: "comma", gdk_name: "comma", display: "," },
    KeyInfo { spec: "period", gdk_name: "period", display: "." },
    KeyInfo { spec: "semicolon", gdk_name: "semicolon", display: ";" },
    KeyInfo { spec: "bracketleft", gdk_name: "bracketleft", display: "[" },
    KeyInfo { spec: "bracketright", gdk_name: "bracketright", display: "]" },
    KeyInfo { spec: "plus", gdk_name: "plus", display: "+" },
    KeyInfo { spec: "minus", gdk_name: "minus", display: "-" },
    KeyInfo { spec: "equal", gdk_name: "equal", display: "=" },
    KeyInfo { spec: "f1", gdk_name: "F1", display: "F1" },
    KeyInfo { spec: "f2", gdk_name: "F2", display: "F2" },
    KeyInfo { spec: "f3", gdk_name: "F3", display: "F3" },
    KeyInfo { spec: "f4", gdk_name: "F4", display: "F4" },
    KeyInfo { spec: "f5", gdk_name: "F5", display: "F5" },
    KeyInfo { spec: "f6", gdk_name: "F6", display: "F6" },
    KeyInfo { spec: "f7", gdk_name: "F7", display: "F7" },
    KeyInfo { spec: "f8", gdk_name: "F8", display: "F8" },
    KeyInfo { spec: "f9", gdk_name: "F9", display: "F9" },
    KeyInfo { spec: "f10", gdk_name: "F10", display: "F10" },
    KeyInfo { spec: "f11", gdk_name: "F11", display: "F11" },
    KeyInfo { spec: "f12", gdk_name: "F12", display: "F12" },
];

pub fn key_info(spec: &str) -> Option<&'static KeyInfo> {
    KEYS.iter().find(|k| k.spec == spec)
}

/// Runtime side of [`KEYS`]: which spec name a pressed keyval corresponds to.
/// Shifted letters and the keypad variants of `+ - 0` fold onto the same spec
/// so a binding matches however the key was produced. `None` for anything not
/// bindable (modifiers, dead keys, IME output).
pub fn keyval_spec(k: gdk::Key) -> Option<&'static str> {
    Some(match k {
        gdk::Key::a | gdk::Key::A => "a",
        gdk::Key::b | gdk::Key::B => "b",
        gdk::Key::c | gdk::Key::C => "c",
        gdk::Key::d | gdk::Key::D => "d",
        gdk::Key::e | gdk::Key::E => "e",
        gdk::Key::f | gdk::Key::F => "f",
        gdk::Key::g | gdk::Key::G => "g",
        gdk::Key::h | gdk::Key::H => "h",
        gdk::Key::i | gdk::Key::I => "i",
        gdk::Key::j | gdk::Key::J => "j",
        gdk::Key::k | gdk::Key::K => "k",
        gdk::Key::l | gdk::Key::L => "l",
        gdk::Key::m | gdk::Key::M => "m",
        gdk::Key::n | gdk::Key::N => "n",
        gdk::Key::o | gdk::Key::O => "o",
        gdk::Key::p | gdk::Key::P => "p",
        gdk::Key::q | gdk::Key::Q => "q",
        gdk::Key::r | gdk::Key::R => "r",
        gdk::Key::s | gdk::Key::S => "s",
        gdk::Key::t | gdk::Key::T => "t",
        gdk::Key::u | gdk::Key::U => "u",
        gdk::Key::v | gdk::Key::V => "v",
        gdk::Key::w | gdk::Key::W => "w",
        gdk::Key::x | gdk::Key::X => "x",
        gdk::Key::y | gdk::Key::Y => "y",
        gdk::Key::z | gdk::Key::Z => "z",
        gdk::Key::_0 | gdk::Key::KP_0 => "0",
        gdk::Key::_1 | gdk::Key::KP_1 => "1",
        gdk::Key::_2 | gdk::Key::KP_2 => "2",
        gdk::Key::_3 | gdk::Key::KP_3 => "3",
        gdk::Key::_4 | gdk::Key::KP_4 => "4",
        gdk::Key::_5 | gdk::Key::KP_5 => "5",
        gdk::Key::_6 | gdk::Key::KP_6 => "6",
        gdk::Key::_7 | gdk::Key::KP_7 => "7",
        gdk::Key::_8 | gdk::Key::KP_8 => "8",
        gdk::Key::_9 | gdk::Key::KP_9 => "9",
        gdk::Key::Up | gdk::Key::KP_Up => "up",
        gdk::Key::Down | gdk::Key::KP_Down => "down",
        gdk::Key::Left | gdk::Key::KP_Left => "left",
        gdk::Key::Right | gdk::Key::KP_Right => "right",
        // Shift+Tab arrives as ISO_Left_Tab on X11 and Wayland alike.
        gdk::Key::Tab | gdk::Key::ISO_Left_Tab | gdk::Key::KP_Tab => "tab",
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => "pageup",
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => "pagedown",
        gdk::Key::Home | gdk::Key::KP_Home => "home",
        gdk::Key::End | gdk::Key::KP_End => "end",
        gdk::Key::Insert | gdk::Key::KP_Insert => "insert",
        gdk::Key::Delete | gdk::Key::KP_Delete => "delete",
        gdk::Key::space | gdk::Key::KP_Space => "space",
        gdk::Key::Return | gdk::Key::KP_Enter => "return",
        gdk::Key::BackSpace => "backspace",
        gdk::Key::Escape => "escape",
        gdk::Key::apostrophe => "apostrophe",
        gdk::Key::slash | gdk::Key::KP_Divide => "slash",
        gdk::Key::backslash => "backslash",
        gdk::Key::grave => "grave",
        gdk::Key::comma => "comma",
        gdk::Key::period => "period",
        gdk::Key::semicolon => "semicolon",
        gdk::Key::bracketleft => "bracketleft",
        gdk::Key::bracketright => "bracketright",
        gdk::Key::plus | gdk::Key::KP_Add => "plus",
        gdk::Key::minus | gdk::Key::KP_Subtract => "minus",
        gdk::Key::equal => "equal",
        gdk::Key::F1 => "f1",
        gdk::Key::F2 => "f2",
        gdk::Key::F3 => "f3",
        gdk::Key::F4 => "f4",
        gdk::Key::F5 => "f5",
        gdk::Key::F6 => "f6",
        gdk::Key::F7 => "f7",
        gdk::Key::F8 => "f8",
        gdk::Key::F9 => "f9",
        gdk::Key::F10 => "f10",
        gdk::Key::F11 => "f11",
        gdk::Key::F12 => "f12",
        _ => return None,
    })
}

/// Build a [`Binding`] from a live key event. `prefix` says whether the chord
/// prefix was armed when the key arrived.
pub fn binding_from_event(
    prefix: bool,
    modifiers: gdk::ModifierType,
    keyval: gdk::Key,
) -> Option<Binding> {
    let key = keyval_spec(keyval)?.to_string();
    Some(Binding {
        prefix,
        // The prefix already accounts for the Ctrl the user is holding.
        ctrl: !prefix && modifiers.contains(gdk::ModifierType::CONTROL_MASK),
        shift: modifiers.contains(gdk::ModifierType::SHIFT_MASK),
        alt: modifiers.contains(gdk::ModifierType::ALT_MASK),
        key,
    })
}

/// Mouse gestures shown alongside the key bindings in Settings. Not rebindable
/// — they're listed so the Keybindings tab is a complete reference.
pub const MOUSE_REFERENCE: &[(&str, &str)] = &[
    ("Middle-click", "Paste primary selection"),
    ("Left-drag", "Select text"),
    ("Double-click / triple-click", "Select word / line"),
    ("Ctrl + Scroll", "Zoom focused pane"),
    ("Right-click", "Open the context menu"),
    ("Toolbar ⋯ drag", "Rearrange pane (drop on an edge of another pane)"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_roundtrip() {
        for spec in ["prefix+down", "ctrl+shift+o", "prefix+ctrl+a", "alt+left", "ctrl+0"] {
            assert_eq!(Binding::parse(spec).unwrap().spec(), spec);
        }
    }

    #[test]
    fn every_default_parses_and_has_a_key() {
        for table in [SKYTERM_DEFAULTS, TERMINATOR_DEFAULTS] {
            for (action, specs) in table {
                for spec in *specs {
                    let b = Binding::parse(spec)
                        .unwrap_or_else(|| panic!("{} / {spec} failed to parse", action.id()));
                    assert!(key_info(&b.key).is_some());
                    assert!(b.is_safe(), "{spec} would shadow a plain keystroke");
                }
            }
        }
    }

    #[test]
    fn keys_table_matches_runtime_matcher() {
        // Anything the table offers must be producible by a real keypress, or
        // Settings would show a binding that can never fire.
        for k in KEYS {
            assert!(
                keyval_spec(gdk::Key::from_name(k.gdk_name).unwrap()) == Some(k.spec),
                "{} does not round-trip through keyval_spec",
                k.spec
            );
        }
    }

    /// No combo may run two different actions. Guards against picking an
    /// already-taken key when adding a binding to either default table.
    #[test]
    fn no_default_binding_is_claimed_twice() {
        for (name, table) in [("skyterm", SKYTERM_DEFAULTS), ("terminator", TERMINATOR_DEFAULTS)] {
            let mut seen: BTreeMap<String, &str> = BTreeMap::new();
            for (action, specs) in table {
                for spec in *specs {
                    if let Some(other) = seen.insert(spec.to_string(), action.id()) {
                        panic!("{name}: {spec} is bound to both {other} and {}", action.id());
                    }
                }
            }
        }
    }

    /// The pair that forced two-pass Ctrl matching: both bindings are on `a`
    /// after the prefix and only Ctrl tells them apart.
    #[test]
    fn select_all_and_send_prefix_are_distinguished_by_ctrl() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        // Ctrl+A then A (Ctrl released) → select all.
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::a),
            Some(Action::SelectAll)
        );
        // Ctrl+A then Ctrl+A (Ctrl held) → literal 0x01 to the shell.
        assert_eq!(
            km.chord_action(gdk::ModifierType::CONTROL_MASK, gdk::Key::a),
            Some(Action::SendPrefix)
        );
    }

    #[test]
    fn clipboard_moved_onto_the_prefix() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        for (key, action) in [
            (gdk::Key::c, Action::Copy),
            (gdk::Key::v, Action::Paste),
        ] {
            // Works with Ctrl still held from the prefix, and without.
            assert_eq!(km.chord_action(gdk::ModifierType::CONTROL_MASK, key), Some(action));
            assert_eq!(km.chord_action(gdk::ModifierType::empty(), key), Some(action));
        }
        // No plain accelerator is left for them in this style.
        assert!(km.accels(Action::Copy).is_empty());
        assert!(km.accels(Action::Paste).is_empty());
        assert!(km.accels(Action::SelectAll).is_empty());
        // Terminator style keeps the familiar Ctrl+Shift combos.
        let term = Keymap::new(ShortcutStyle::Terminator, &BTreeMap::new());
        assert_eq!(term.accels(Action::Copy), vec!["<Primary><Shift>c".to_string()]);
    }

    #[test]
    fn menu_and_shortcut_reference_chords() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::Return),
            Some(Action::ShowMenu)
        );
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::BackSpace),
            Some(Action::ShowShortcuts)
        );
    }

    #[test]
    fn window_state_chords_are_the_brackets() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        assert_eq!(
            km.chord_action(gdk::ModifierType::CONTROL_MASK, gdk::Key::bracketright),
            Some(Action::MaximizeWindow)
        );
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::bracketleft),
            Some(Action::MinimizeWindow)
        );
        // Terminator style: F11 maximizes, minimize has no equivalent.
        let term = Keymap::new(ShortcutStyle::Terminator, &BTreeMap::new());
        assert_eq!(term.accels(Action::MaximizeWindow), vec!["F11".to_string()]);
        assert!(term.bindings(Action::MinimizeWindow).is_empty());
    }

    #[test]
    fn focus_pane_is_prefix_f() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        assert_eq!(
            km.chord_action(gdk::ModifierType::CONTROL_MASK, gdk::Key::f),
            Some(Action::FocusPane)
        );
        // Terminator binds it to a plain accelerator instead, so no chord.
        let term = Keymap::new(ShortcutStyle::Terminator, &BTreeMap::new());
        assert!(term.chord_action(gdk::ModifierType::empty(), gdk::Key::f).is_none());
        assert_eq!(
            term.accels(Action::FocusPane),
            vec!["<Primary><Shift>x".to_string()]
        );
    }

    #[test]
    fn skyterm_style_uses_the_prefix_terminator_does_not() {
        let empty = BTreeMap::new();
        assert!(Keymap::new(ShortcutStyle::Skyterm, &empty).prefix_enabled());
        assert!(!Keymap::new(ShortcutStyle::Terminator, &empty).prefix_enabled());
    }

    #[test]
    fn close_pane_is_prefix_x() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        assert_eq!(
            km.chord_action(gdk::ModifierType::CONTROL_MASK, gdk::Key::x),
            Some(Action::ClosePane)
        );
        // Ctrl still held from the prefix must not block a non-Ctrl binding.
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::X),
            Some(Action::ClosePane)
        );
    }

    #[test]
    fn shift_distinguishes_next_and_prev_tab() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::Tab),
            Some(Action::NextTab)
        );
        assert_eq!(
            km.chord_action(gdk::ModifierType::SHIFT_MASK, gdk::Key::ISO_Left_Tab),
            Some(Action::PrevTab)
        );
    }

    #[test]
    fn custom_overrides_layer_on_skyterm_defaults() {
        let mut custom = BTreeMap::new();
        custom.insert("close".to_string(), "prefix+q".to_string());
        custom.insert("new-tab".to_string(), String::new());
        let km = Keymap::new(ShortcutStyle::Custom, &custom);
        assert_eq!(
            km.chord_action(gdk::ModifierType::empty(), gdk::Key::q),
            Some(Action::ClosePane)
        );
        assert!(km.bindings(Action::NewTab).is_empty());
        // Untouched actions keep the skyterm default.
        assert_eq!(km.bindings(Action::SplitDown)[0].spec(), "prefix+down");
    }

    #[test]
    fn accels_skip_prefix_chords() {
        let km = Keymap::new(ShortcutStyle::Skyterm, &BTreeMap::new());
        assert!(km.accels(Action::SplitDown).is_empty());
        assert_eq!(
            km.accels(Action::NewWindow),
            vec!["<Primary><Shift>n".to_string()]
        );
    }

    #[test]
    fn snapshot_round_trips_through_custom() {
        let km = Keymap::new(ShortcutStyle::Terminator, &BTreeMap::new());
        let snap = km.snapshot();
        let custom = Keymap::new(ShortcutStyle::Custom, &snap);
        for a in Action::all() {
            assert_eq!(km.bindings(*a), custom.bindings(*a), "{}", a.id());
        }
    }
}
