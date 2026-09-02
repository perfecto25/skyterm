# Skyterm terminal emulator


> "Everything should be obvious and easy to use." 
>
> \- some lazy developer (me)

![Skyterm](skyterm-gui/resources/skyterm_sm.png)


Skyterm is basic terminal emulator inspired by great terminal emulators like Terminator and Tilix

I built it because I wanted a simple terminal emulator with all my requirements included:

1. Baked in themes - I dont want to download themes from different places and spend time configuring them. I want a terminal to have 10-20 solid light and dark themes that I can start using off the bat.

1. Obvious menu and shortcuts - one annoying feature of many new GPU terminals is lack of right click menu. I dont want to read the docs and remember various keyboard shortcuts. The menu should be simple, obvious and easy to use - and also provide keyboard shortcuts for same actions

1. Lean and mean - the terminal should be as minimal on resource usage as possible, and still have extremely fast performance and rendering using GPU rendering if available

1. The focus of the terminal should be out of box, batteries included functionality for 99% of required terminal workload, ie, ssh, panes, tabs, splitting, fonts, themes, for anything more advanced, there will be an option to customize. But once you install Skyterm - it should come with usable sane defaults and options for vast majority of users


Skyterm's aim is to be light weight, low-resource and fast.

It uses GPU rendering with OpenGL for fast performance.

Skyterm is written with Claude code and all code is human reviewed and tested.  

Skyterm is written in Rust for performance, memory safety and availability of large number of terminal application libs.


## Features

Skyterm has the following features:

- Tabs (renameable, draggable, with preset pane layouts)
- Panes (ability to split a tab into multiple panes, drag them into new positions or into another tab)
- Focus mode - enlarge the pane you are working in without losing the others
- Infinite scrollback
- built in Themes (future release will add Terminator-compatible theme files)
- shortcut key bindings - Skyterm or Terminator style, or record your own, with a searchable reference built in


Skyterm aims to be basic, fast and no-nonsense terminal emulator tailored for system administrators, developers and anyone who wants a responsive and lightweight terminal that doesnt get in your way. 

## Keyboard Actions

all Skyterm actions are available through a menu (right click to open menu) or through a keyboard shortcut.

Every shortcut is listed in Menu > Settings > Keybindings, with a search box - type part of an action or a combo ("split", "tab", "ctrl") to filter the list. The same list is a keypress away at any time with `Ctrl+A → Backspace` (Menu > Shortcuts…).

The menu shows its grey shortcut hints only when every item can show one. Under Skyterm style most shortcuts are two-key `Ctrl+A` chords, which GTK cannot draw as an accelerator, so the hints are hidden rather than shown for a random four items; under Terminator style they all appear.

#### Shortcut Style

The **Shortcut Style** drop-down at the top of Settings > Keybindings picks which set of shortcuts is active:

- **Skyterm** (default) - the `Ctrl+A` prefix bindings documented below.
- **Terminator** - Terminator-compatible bindings (`Ctrl+Shift+O` / `Ctrl+Shift+E` to split, `Ctrl+Shift+W` to close, `Ctrl+PageUp` / `Ctrl+PageDown` for tabs, `Alt`+arrows to move focus). The `Ctrl+A` prefix is disabled in this style, so `Ctrl+A` reaches the shell as readline's beginning-of-line.
- **Custom** - starts as a copy of the set you were using; click any combo to record a replacement, or ✕ to unbind it. A combo needs `Ctrl`, `Alt` or the `Ctrl+A` prefix so it can't shadow ordinary typing.

The choice is saved to `config.toml` as `shortcut_style`, and custom combos as a `[keybindings]` table.

#### All Skyterm shortcuts (default style)

`Ctrl+A → X` means: press `Ctrl+A`, release, then press `X`. You have 2 seconds to hit the second key. On macOS the prefix is `Command+A`.

| Shortcut | Action |
| --- | --- |
| `Ctrl+A` → `↓` | Split pane down (new pane below) |
| `Ctrl+A` → `→` | Split pane right (new pane to the right) |
| `Ctrl+A` → `↑` | Split pane up (new pane above) |
| `Ctrl+A` → `←` | Split pane left (new pane to the left) |
| `Ctrl+A` → `F` | Focus pane - enlarge the active pane, press again to restore |
| `Ctrl+A` → `X` | Close the focused pane |
| `Ctrl+A` → `T` | Open a new tab |
| `Ctrl+Shift+N` | Open a new window |
| `Ctrl+A` → `]` | Maximize the window (press again to restore) |
| `Ctrl+A` → `[` | Minimize the window |
| `Ctrl+A` → `Tab` | Next tab |
| `Ctrl+A` → `Shift+Tab` | Previous tab |
| `Ctrl+A` → `O` | Cycle focus to the next pane |
| `Ctrl+A` → `H` | Focus the pane to the left |
| `Ctrl+A` → `J` | Focus the pane below |
| `Ctrl+A` → `K` | Focus the pane above |
| `Ctrl+A` → `L` | Focus the pane to the right |
| `Ctrl+A` → `C` | Copy selection |
| `Ctrl+A` → `V` | Paste from system clipboard |
| `Ctrl+A` → `A` | Select all (focused pane) |
| `Ctrl+A` → `Enter` | Open the context menu (same as right-click) |
| `Ctrl+A` → `Backspace` | Show this shortcut reference |
| `Ctrl+A` → `'` | Previous theme (focused pane) |
| `Ctrl+A` → `/` | Next theme (focused pane) |
| `Ctrl` + `+` / `Ctrl` + `=` | Zoom in (focused pane) |
| `Ctrl` + `-` | Zoom out (focused pane) |
| `Ctrl` + `0` | Reset font size (focused pane) |
| `Ctrl+A` → `Ctrl+A` | Send a literal `Ctrl+A` to the shell (readline beginning-of-line) |

Note the last two rows: `Ctrl+A` → `A` (Ctrl released) selects all, while `Ctrl+A` → `Ctrl+A` (Ctrl held down) sends the literal `Ctrl+A` through to the shell. Holding Ctrl for the second key is fine everywhere else - it only changes the meaning of this one pair.

#### All Terminator shortcuts

Active when Shortcut Style is set to **Terminator**. The `Ctrl+A` prefix is switched off in this style, so `Ctrl+A` goes straight to the shell.

| Shortcut | Action |
| --- | --- |
| `Ctrl+Shift+O` | Split pane down (new pane below) |
| `Ctrl+Shift+E` | Split pane right (new pane to the right) |
| `Ctrl+Shift+X` | Focus pane - enlarge the active pane, press again to restore |
| `Ctrl+Shift+W` | Close the focused pane |
| `Ctrl+Shift+T` | Open a new tab |
| `Ctrl+Shift+I` | Open a new window |
| `F11` | Maximize the window (press again to restore) |
| `Ctrl+PageDown` | Next tab |
| `Ctrl+PageUp` | Previous tab |
| `Ctrl+Tab` | Cycle focus to the next pane |
| `Alt` + `←` / `↓` / `↑` / `→` | Focus the pane in that direction |
| `Ctrl+Shift+C` | Copy selection |
| `Ctrl+Shift+V` | Paste from system clipboard |
| `Ctrl+Shift+A` | Select all (focused pane) |
| `Ctrl+Shift+[` | Previous theme (focused pane) |
| `Ctrl+Shift+]` | Next theme (focused pane) |
| `Ctrl` + `+` / `Ctrl` + `=` | Zoom in (focused pane) |
| `Ctrl` + `-` | Zoom out (focused pane) |
| `Ctrl` + `0` | Reset font size (focused pane) |

Terminator has no equivalent for splitting up / left, minimizing the window, opening the menu from the keyboard, or the shortcut reference, so those actions are unbound in this style - switch to **Custom** if you want them back. The menu is always a right-click away, and the reference is on the menu as Shortcuts…

#### MacOS shortcuts

On macOS `Ctrl` belongs to the shell (`Ctrl+A` is beginning-of-line, `Ctrl+C` is SIGINT), so Skyterm puts its actions on `Command`. These work in every shortcut style:

| Shortcut | Action |
| --- | --- |
| `Command+A` → key | Chord prefix - every `Ctrl+A` chord above becomes `Command+A` |
| `Command+D` | Split pane right |
| `Command+Shift+D` | Split pane down |
| `Command+T` | Open a new tab |
| `Command+O` | Cycle focus to the next pane |
| `Command+Option` + arrows | Focus the pane in that direction |
| `Command+C` | Copy selection |
| `Command+V` | Paste from system clipboard |
| `Command` + `+` / `=` | Zoom in |
| `Command` + `-` | Zoom out |
| `Command` + `0` | Reset font size |

#### Mouse

| Gesture | Action |
| --- | --- |
| Left-drag | Select text |
| Double-click / triple-click | Select word / line |
| Middle-click | Paste primary selection |
| Right-click | Open the context menu |
| `Ctrl` + scroll | Zoom the focused pane |
| Toolbar `⋯` drag | Rearrange pane (drop on an edge of another pane, or on a tab) |

While an application has mouse reporting on (htop, vim, less), clicks and drags go to that application - hold `Shift` to select text locally instead.


#### Zoom

to zoom in on pane content 

```
Linux

    Ctrl + "+" (zoom in)
    Ctrl + "-" (zoom out)
    Ctrl + "0" (reset to the configured font size)
```

```
MacOS

    Command + "+" (zoom in)
    Command + "-"  (zoom out)
    Command + "0"  (reset to the configured font size)
```

or with mouse

```
Linux

    Ctrl + mouse scroll up (zoom in)
    Ctrl + mouse scroll down (zoom out)
```

```
MacOS

    Command + mouse scroll up (zoom out)
    Command + mouse scroll down (zoom in)
```

Zoom is per-pane - only the focused pane changes.

#### Tabs

open new Tab

```
Linux

    Ctrl+A → T
```

```
MacOS

    Command + A → T
```

switch tabs

```
Linux

    Ctrl+A → Tab          (next tab)
    Ctrl+A → Shift+Tab    (previous tab)
```

```
MacOS

    Command + A → Tab          (next tab)
    Command + A → Shift+Tab    (previous tab)
```

**rename a Tab**

by default, each tab will be named Tab + incremented number, to change the name of each tab, double click on Tab header and type in a new name, hit Enter

#### Panes and Windows

Panes are virtual sub-terminals inside the parent terminal window

Split panes

```
Linux

    Ctrl+A → Right key  (split pane right)
    Ctrl+A → Left key  (split pane left)
    Ctrl+A → Up key  (split pane up)
    Ctrl+A → Down key  (split pane down)
```

```
MacOS

    Command+A → Right key  (split pane right)
    Command+A → Left key  (split pane left)
    Command+A → Up key  (split pane up)
    Command+A → Down key  (split pane down)

    Command+D            (split pane right)
    Command+Shift+D      (split pane down)
```

Pane cycling

```
Linux

    Ctrl+A → o	Cycle to the next pane in the tab
    Ctrl+A → h	Focus pane to the left
    Ctrl+A → j	Focus pane below
    Ctrl+A → k	Focus pane above
    Ctrl+A → l	Focus pane to the right
```

```
MacOS

    Command+A → o	Cycle to the next pane in the tab
    Command+A → h	Focus pane to the left
    Command+A → j	Focus pane below
    Command+A → k	Focus pane above
    Command+A → l	Focus pane to the right

    Command+O            Cycle to the next pane in the tab
    Command+Option+←↓↑→  Focus pane in that direction
```

Focus pane

when a tab has 2 or more panes, focus mode enlarges the active pane to 90% of the tab and squeezes the others down to a sliver at the edge, so you get room to work without losing track of what else is open. Hitting the shortcut again puts every divider back exactly where it was. There is no animation - the new layout is applied on the next frame.

```
Linux

    Ctrl+A → F
```

```
MacOS

    Command+A → F
```

also available from the right click Menu > Focus Pane.

Only the enlarged pane is resized, so only its shell sees the new width - the squeezed panes keep their contents at the size they had, and you see the top-left corner of each until you toggle back. Focus mode ends by itself if you move focus to another pane, close a pane, split, or drag a pane somewhere else.

Close pane

```
Linux

    Ctrl+A → X
```

```
MacOS
    
    Command+A → X
``` 

Panes can be dragged into new positions or moved to another tab via the Pane toolbar visible on top right corner of each pane.

To move a pane to another tab, drag the pane via Toolbar drag icon and hover over the tab you want to place it, then drop it into a position inside this tab.

Pane toolbars can be disabled in Settings > Behavior or via config file variable 'show_pane_toolbar'

To open a new terminal window, right click Menu > New Window or via shortcut

    Linux    Shift + Ctrl + N
    MacOS    Shift + Command + N

**Maximize and minimize**

Maximize grows the window to the full size of the monitor it is on, and hitting the same shortcut again puts it back to the size it was. Minimize sends it to the taskbar - bring it back the way your desktop normally does, since a minimized window has no keyboard to listen to.

```
Linux

    Ctrl+A → ]    (maximize / restore)
    Ctrl+A → [    (minimize)
```

```
MacOS

    Command+A → ]    (maximize / restore)
    Command+A → [    (minimize)
```

both are also on the right click Menu > Maximize Window / Minimize Window.

#### Copy and paste

```
Linux

    Ctrl+A → C    (copy)
    Ctrl+A → V    (paste)
    Ctrl+A → A    (select all)
```

```
MacOS

    Command+A → C    (copy)
    Command+A → V    (paste)
    Command+A → A    (select all)

    Command + C      (copy)
    Command + V      (paste)
```

`Shift + Ctrl + C` / `V` / `A` are the Terminator style bindings for these - switch Shortcut Style in Settings > Keybindings if you prefer them.

Middle-click pastes the primary selection (the text you last selected with the mouse, no copy needed).

Selecting text can also copy it automatically - turn on 'copy on select' in Settings > Behavior.

#### Menu and shortcut reference from the keyboard

The right click menu can be opened without the mouse - it appears in the middle of the focused pane:

```
Linux            Ctrl+A → Enter
MacOS            Command+A → Enter
```

And the shortcut reference - the same searchable list as Settings > Keybindings, on its own so you can leave it open next to your work:

```
Linux            Ctrl+A → Backspace
MacOS            Command+A → Backspace
```

also on the right click Menu > Shortcuts… . Escape closes it.


#### Color Themes

Themes can be applied globally to all tabs and panes by going to Menu > Settings > Theme and choosing a color theme

To apply to specific pane, right click on the pane, Menu > Themes > choose a theme - this will be applied locally to the pane

You can also scroll through available themes and see how they look on the pane:

```
Linux

    hold down Ctrl + A and hit ' or / to cycle up or down a theme
```

```
MacOS

    hold down Command + A and hit ' or / to cycle up or down a theme
```

**Custom Themes**

to add your own custom theme, place a theme-name.toml file into 

    ~/.config/skyterm/themes 
    
directory


custom themes are declared in a TOML format

    [themes]
    [themes.MyTheme1]
    palette = "#353535:#d25252:#a5c261:#ffc66d:#6c99bb:#d197d9:#bed6ff:#eeeeec:#535353:#f00c0c:#c2e075:#e1e48b:#8ab7d9:#efb5f7:#dcf4ff:#ffffff"
    background_color = "#323232"
    cursor_color = "#d6d6d6"
    foreground_color = "#ffffff"
    font_size = 12
    font_path = "/usr/share/fonts/myfont/myfont.ttf"

    [themes.SomeOtherTheme]
    cursor_color = "#BAFFAA"
    foreground_color = "#FFFFFF"
    palette = "#444444:#FF0054:#B1D630:#9D895E:#67BEE3:#B576BC:#569A9F:#EDEDED:#777777:#D65E75:#BAFFAA:#ECE1C8:#9FD3E5:#DEB3DF:#B6E0E5:#FFFFFF"

Custom themes will show up in Menu > Themes > Custom


### Custom configuration

additional configration can be applied to your skyterm config file 

located in ~/.config/skyterm/config.toml

    font_path = ":embedded:JetBrainsMono-Regular:"
    font_size = 16
    theme_name = "Skyterm Blue"
    scrollback_lines = 10000
    cursor_blink = true
    click_word_select = true
    copy_on_select = false
    shortcut_style = "skyterm"

a custom font path can be added to font_path variable

theme_name is the default theme applied to all panes and tabs

click_word_select toggles double-click-to-select-word and triple-click-to-select-line (set to false to disable). Also available under Settings > Behavior.

copy_on_select automatically copies any selection (word, line, or drag) to the clipboard as soon as you make it (set to true to enable). Also available under Settings > Behavior.

shortcut_style picks the active keyboard shortcut set - `skyterm`, `terminator` or `custom`. Also available under Settings > Keybindings. With `custom`, each action's combo lives in a `[keybindings]` table written by Settings:

    shortcut_style = "custom"

    [keybindings]
    split-horizontal = "prefix+down"
    split-vertical = "prefix+right"
    close = "prefix+x"
    new-tab = "ctrl+shift+t"
    copy = "ctrl+shift+c"

`prefix` means "after the Ctrl+A prefix", so `prefix+down` is `Ctrl+A → Down`. An empty value leaves the action unbound, and any action you leave out of the table falls back to its Skyterm default. Combos are easier to record in Settings > Keybindings than to hand-write.

for specific theme on a specific pane, right click > Menu > Themes and choose a theme to apply to this specific pane



### Layouts

Skyterm supports preconfigured pane layouts

Available layouts 

single 

```
+----------------+ 
|                | 
|                |
|                |
|                |
+----------------+ 
```

2 panes vertical

```
+------+ +------+
|      | |      |
|      | |      |
|      | |      |
|      | |      |
+------+ +------+
```

2 panes horizontal

```
+----------------+
|                |
|                |
+----------------+
+----------------+
|                |
|                |
+----------------+
```

3 panes

```
+---------+ +---------+
|         |  |        |
|         |  |        |
+---------+ +---------+
+---------------------+
|                     |
|                     |
+---------------------+
```

4 panes

```
+---------+ +---------+
|         |  |        |
|         |  |        |
+---------+ +---------+
+---------+ +---------+
|         |  |        |
|         |  |        |
+---------+ +---------+
```

Pane layout can also be configured in skyterm config TOML (default is single pane)

    default_layout = single / 2v / 2h / 3 / 4

or from the Menu > Settings > Behavior > Default layout

### Building


cargo build --release

binary is located in target/release/skyterm

for RPM builds

    cargo install cargo-generate-rpm
    cargo build --release -p skyterm-gui
    cargo generate-rpm -p skyterm-gui

or run ./package-rpm.sh

    Install:    sudo rpm -ivh target/generate-rpm/skyterm-0.1.1-1.x86_64.rpm
    Upgrade:    sudo rpm -Uvh target/generate-rpm/skyterm-0.1.1-1.x86_64.rpm
    Verify:     rpm -qip target/generate-rpm/skyterm-0.1.1-1.x86_64.rpm


for DEB builds

    ./package-deb.sh


for Arch Linux builds

on Arch, the PKGBUILD in the repo root builds from the working tree

    makepkg -f

    Install:    sudo pacman -U skyterm-0.1.8-1-x86_64.pkg.tar.zst
    Upgrade:    same command - pacman replaces the installed version
    Verify:     pacman -Qip skyterm-0.1.8-1-x86_64.pkg.tar.zst

the version comes from Cargo.toml, so there is nothing to bump in the PKGBUILD. On a non-Arch machine you can build the package in a container

    podman run --rm -v "$PWD":/repo:ro,Z archlinux:base-devel bash -c '
      pacman -Syu --noconfirm --needed git rust gtk4 libepoxy freetype2 fontconfig pkgconf sudo
      useradd -m builder && cp -a /repo /build && chown -R builder:builder /build
      cd /build && sudo -u builder makepkg -f --noconfirm --nodeps'


for MacOS builds

    ./package-macos.sh


### Releases

Releases are built by `.github/workflows/release.yml` on GitHub's free runners. Push a version tag and every artifact is built and attached to a GitHub Release:

    git tag -a v0.1.8 -m 'release 0.1.8'
    git push origin --tags

| Job | Runner | Artifact |
| --- | --- | --- |
| `linux-x86_64` | ubuntu-24.04 | `.deb` + `.rpm` |
| `linux-aarch64` | ubuntu-24.04-arm | `.deb` + `.rpm` |
| `arch-x86_64` | archlinux container on ubuntu-24.04 | `.pkg.tar.zst` |
| `macos-arm64` | macos-14 | `.dmg` |

skyterm links GTK4 dynamically, so every target is built natively rather than cross-compiled - that is also why the Arch package is built inside the official `archlinux:base-devel` container instead of on the Ubuntu runner.

Run the workflow manually (Actions > release > Run workflow) to build and upload the artifacts without publishing a release - useful for testing the pipeline before cutting a tag.

To re-run a release for an existing tag

    git tag -d v0.1.8
    git tag v0.1.8
    git push --force origin v0.1.8


### Changelog

#### 0.1.8

- add Focus Pane option
  
  - Control + A + F (makes current pane take up 90% of screen size) - similar to Waveterm Focus feature
  - also on the right click Menu, and rebindable under Settings > Keybindings

- update keybindings:
  
  - add shortcut search under Settings > Keybindings
  - split shortcuts by Style ie, Skyterm style, Terminator style, Custom style
  - Menu shortcuts helpers should be dynamic - based on selected shortcut style - the grey hints only show when every menu item can show one, so they are hidden under Skyterm style (Ctrl+A chords cant be drawn as an accelerator) and shown under Terminator style
  - add Maximize/Minimize terminal shortcut - Control + A + ] and Control + A + [ , also on the right click Menu 
  - add Control + A + Backspace to open up direct Keyboard shortcut lookup window - also on the right click Menu > Shortcuts…
  - move Copy / Paste / Select All onto the prefix - Control + A + C / V / A 
  - add Control + A + Enter to open the right click Menu from the keyboard 

- Builds:

  - add arch linux build (done) - PKGBUILD in the repo root, plus an `arch-x86_64` job in the release workflow that builds it in the official archlinux container and publishes the .pkg.tar.zst with the release


#### 0.1.7
- cycle tabs using Control + Tab (done)
- fix Macos small pane size jitter bug
- linux - ncdu not working, not scrolling up or down (done)


#### 0.1.6

- tabs are wider in size and can be dragged left or right
- maximum number of tabs by default is set to 20 for performance reasons. To change this number, update tab_max_number variable in config.toml
- when trying to create new tab after max # of tab limit reached, a warning banner appears and explains why new tab isnt being opened
- closing pane, tab or window now throws a confirmation prompt, can be disabled via confirm_ variables in config file

    confirm_tab_close = true
    confirm_pane_close = true
    confirm_window_close = true

- panes are now draggable, each pane has a small tooltip on top right, with Drag and Close buttons. Panes can be dragged into new horizontal or vertical position, when you hover over a new position, the position is highlighted with green color
- added "new window" command to spawn off a new terminal window
- can drag a pane into a different tab
- custom themes can now be added via toml theme files

### Roadmap

- ~~add documentation on themes, add ability to integrate custom themes via terminator-style config files~~
- ~~add New Window command~~
- ~~change split menu option to single row with 4 sub buttons Up, Down, Left, Right~~
~~- add keyboard shortcut helpers to all menu commands (gray shortcut helper text)~~
- add button few pixels to right of rightmost tab "new tab", should be able to create new tab via button (along w menu and KB shortcut)
- ~~add ability to drag and drop panes in different locations, ie terminator behavior, move panes left, right etc~~
- ~~add ability to drag a pane into a different tab, highlight the drop placement area on the new tab~~

