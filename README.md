# Skyterm terminal emulator

Skyterm is basic terminal emulator inspired by Terminator.

its aim is to be light weight, low-resource and fast.

It uses GPU rendering with OpenGL for fast performance.

Skyterm is written with Claude code and all code is human reviewed and tested.  

Skyterm is written in Rust for performance, memory safety and availability of large number of terminal application libs.


## Features

Skyterm has the following features:

- Tabs
- Panes (ability to split a tab into multiple panes)
- Infinite scrollback
- Themes (compatible with Terminator theme files)
- shortcut key bindings


Skyterm aims to be basic, fast and no-nonsense terminal emulator tailored for system administrators, developers and anyone who wants a responsive and lightweight terminal that doesnt get in your way. 

## Keyboard Actions

all Skyterm actions are available through a menu (right click to open menu) or through a keyboard shortcut.

#### Zoom

to zoom in on pane content 

    Control + "+" (zoom in)
    Control + "-" (zoom out)

or with mouse

    Control + mouse scroll up
    Control + mouse scroll down

#### Tabs

open new Tab

    Ctrl+A → T


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



### Building


cargo build --release

binary is located in target/release/skyterm


### Roadmap

- add documentation on themes
- add Info section to menu that shows skyterm version
