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

- Tabs
- Panes (ability to split a tab into multiple panes)
- Infinite scrollback
- built in Themes (future release will add Terminator-compatible theme files)
- shortcut key bindings


Skyterm aims to be basic, fast and no-nonsense terminal emulator tailored for system administrators, developers and anyone who wants a responsive and lightweight terminal that doesnt get in your way. 

## Keyboard Actions

all Skyterm actions are available through a menu (right click to open menu) or through a keyboard shortcut.


#### Zoom

to zoom in on pane content 

```
Linux

    Ctrl + "+" (zoom in)
    Ctrl + "-" (zoom out)
```

```
MacOS

    Command + "+" (zoom in)
    Command + "-"  (zoom out)
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

**rename a Tab**

by default, each tab will be named Tab + incremented number, to change the name of each tab, double click on Tab header and type in a new name, hit Enter

#### Panes and Windows

Panes are virtual sub-terminals inside the parent terminal window

Split panes

```
Linux

    Ctrl+A → Right key  (split pane right)
    Ctrl+A → Left key  (split pane left)
    Ctrl+A → Up key  (split pane right)
    Ctrl+A → Down key  (split pane down)

    or Terminator-style shortcuts

    Shift + Ctrl + O (split horizontally)
    Shift + Ctrl + E (split vertically)
```

```
MacOS

    Command+A → Right key  (split pane right)
    Command+A → Left key  (split pane left)
    Command+A → Up key  (split pane right)
    Command+A → Down key  (split pane down)
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
```

Panes can be dragged into new positions or moved to another tab via the Pane toolbar visible on top right corner of each pane.

To move a pane to another tab, drag the pane via Toolbar drag icon and hover over the tab you want to place it, then drop it into a position inside this tab.

Pane toolbars can be disabled in Settings > Behavior or via config file variable 'show_pane_toolbar'

To open a new terminal window, right click Menu > New Window or via shortcut

    Shift + Ctrl + N

#### Copy and paste

Copy 

    Shift + Ctrl + C

Paste 

    Shift + Ctrl + V

Select All

    Shift + Ctrl + A


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

a custom font path can be added to font_path variable

theme_name is the default theme applied to all panes and tabs

click_word_select toggles double-click-to-select-word and triple-click-to-select-line (set to false to disable). Also available under Settings > Behavior.

copy_on_select automatically copies any selection (word, line, or drag) to the clipboard as soon as you make it (set to true to enable). Also available under Settings > Behavior.

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


for MacOS builds

    ./package-macos.sh


### Changelog

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