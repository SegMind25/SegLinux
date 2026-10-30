# seglinux-compositor (preview)

SegLinux's own Wayland compositor, written in Rust on
[Smithay](https://github.com/Smithay/smithay) (`crates/seglinux-compositor`).
It is meant to replace labwc. Until it does everything labwc does, labwc
stays the default and this is an extra choice on the sign-in screen:
**SegLinux (new compositor, preview)**.

![The SegLinux wallpaper, a terminal and a panel listing the open windows, on seglinux-compositor](images/compositor-preview.png)

*Tested in a window, with Waybar standing in for the SegLinux dock (the same
two protocols: layer-shell and foreign-toplevel).*

## What works

- Application windows (xdg-shell): placed in the middle of the screen,
  focus on click, move and resize from the title bar, Super+drag moves a
  window from anywhere in it; maximize, fullscreen, minimize, snap to the
  left or right half.
- The SegLinux desktop's wallpaper, dock and menus (wlr-layer-shell), with
  windows kept out of the dock's space; the dock's open windows
  (wlr-foreign-toplevel): click to switch, minimize or restore, close, and
  show the desktop.
- Copy and paste (also middle-click paste), GPU buffers for games and
  OpenGL/Vulkan applications (linux-dmabuf), several screens side by side
  and plugging screens in and out, switching terminals (Ctrl+Alt+F1..F12).
- The same keys as the labwc session (below), and the same start-up: the
  session runs `/usr/share/seglinux/labwc/autostart` (sound, the password
  agent, the desktop).

Not yet: title bars
for applications that do not draw their own, XWayland (older X11
applications and many games through Wine/Proton), workspaces, screen
sharing and screenshots (wlr-screencopy), per-screen scaling and refresh
choice, tearing for games, touch. These come in the next steps; labwc
remains the default until they are done.

## Settings

`~/.config/seglinux/compositor.toml`; every key is optional. Super+Shift+R
reads it again.

```toml
# Replaces the default start-up; keep the first entry for the desktop.
autostart = ["sh /usr/share/seglinux/labwc/autostart", "nm-applet"]
background = "#0b1220"
focus_follows_mouse = false

[keyboard]
layout = "us,ara"
options = "grp:alt_shift_toggle"
repeat_delay = 300
repeat_rate = 30

[binds]
"Super+Return" = "spawn foot"
"Super+q" = "close"
"Alt+Tab" = "none"          # gives Alt+Tab back to applications
```

Actions: `spawn COMMAND`, `close`, `maximize`, `fullscreen`, `minimize`,
`cycle-windows`, `snap-left`, `snap-right`, `show-desktop`,
`reload-config`, `quit`, `switch-vt N`. Keys are xkb names (`Return`,
`F4`, `Print`, `XF86AudioMute`) with `Super`, `Ctrl`, `Alt` and `Shift`.

Default keys: Super+Space Start, Super+E files, Super+Return terminal,
Super+I settings, Super+L lock, Super+D desktop, Super+Left/Right snap,
Super+Up maximize, Super+Down minimize, Super+F11 fullscreen, Alt+F4 close,
Alt+Tab next window, Print / Shift+Print / Super+Shift+S screenshots,
Super+. emoji, the volume and brightness keys, Ctrl+Alt+Backspace quit.

## Trying it

From the sign-in screen: choose **SegLinux (new compositor, preview)**. In
a window of a running desktop: `seglinux-compositor --winit`. Problems are
logged with `RUST_LOG=debug`. If the session does not start, sign in to
**SegLinux** as before: nothing of labwc's session was changed.
