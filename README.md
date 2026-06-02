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

    Ctrl + "+" (zoom in)
    Ctrl + "-" (zoom out)

or with mouse

    Ctrl + mouse scroll up
    Ctrl + mouse scroll down

#### Tabs

open new Tab

    Ctrl+A → T

rename a Tab

by default, each tab will be named Tab + incremented number, to change the name of each tab, double click on Tab header and type in a new name, hit Enter

#### Panes

Split panes

    Ctrl+A → Right key  (split pane right)
    Ctrl+A → Left key  (split pane left)
    Ctrl+A → Up key  (split pane right)
    Ctrl+A → Down key  (split pane down)

Pane cycling

    Ctrl+A → o	Cycle to the next pane in the tab
    Ctrl+A → h	Focus pane to the left
    Ctrl+A → j	Focus pane below
    Ctrl+A → k	Focus pane above
    Ctrl+A → l	Focus pane to the right


#### Copy and paste

Copy 

    Shift + Ctrl + c

Paste 

    Shift + Ctrl + v

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

### Roadmap

- add documentation on themes, add ability to integrate custom themes via terminator-style config files
- add ability to view images in terminal
- add syntax highlighter for bash, python, json, yaml via cat
