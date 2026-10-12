# Wincraft Rust desktop

This is the first implementation of Wincraft's own desktop shell. The shell
is Rust with GTK4 Rust bindings; it is not a rewrite of the entire desktop
stack in Rust. GTK, labwc, SDDM and the bundled applications retain their
upstream implementations. labwc handles composition, decorations, snapping,
Alt+Tab and workspaces. Plasma remains an alternative session.

## Implemented

- Original dark wallpaper and vector Wincraft mark; dark Plasma fallback.
- Wallpaper surfaces and a bottom panel on every connected display.
- An application launcher populated through GIO's desktop-entry database,
  with case-insensitive multiword search and normal desktop-entry launching.
- Pinned Files, browser and terminal shortcuts.
- Running-window buttons using the Wayland foreign-toplevel protocol;
  click to activate or minimize a window.
- A local clock, desktop settings and links to sound, network and display tools.
- Screen locking, with confirmation before signing out or powering off.
- A live-session installer shortcut and unattended-installer startup.
- A separate SDDM session, PipeWire startup, polkit agent and portal preferences.

## Try it without switching desktop

```sh
cargo run -p wincraft-shell -- --preview --wallpaper "$PWD/data/artwork/wincraft-dark.png"
```

Preview mode intentionally has no running-window integration and disables
locking and power-off. Its launcher and shortcuts open real installed apps.
Use the packaged **Wincraft (Rust)** login session for the Wayland desktop.
A compositor supporting layer-shell and foreign-toplevel is required.

## Shortcuts

| Shortcut | Action |
| --- | --- |
| Super+Space | Application launcher |
| Super+E | Files |
| Super+Enter | Terminal |
| Super+L | Lock |
| Super+D | Show desktop |
| Super+Left / Right | Snap a window |
| Alt+Tab | Window switcher |
| Volume keys | PipeWire volume and mute |

## Preferences

Run `wincraft-shell --settings`, or open Wincraft Desktop Settings from Start.
Choose an existing image by absolute path and a 12-hour or 24-hour clock.
Changes apply next session. For other clock formats, copy
`/usr/share/wincraft/shell.example.toml` to `~/.config/wincraft/shell.toml`.
`XDG_CONFIG_HOME` is respected. Invalid configurations log a warning and use
defaults. Settings are saved atomically.

The compositor uses `/usr/share/wincraft/labwc`. These distribution defaults
are deliberately independent of any existing personal labwc configuration.
The session does not run arbitrary XDG autostart entries yet; its startup
script explicitly launches the components shipped with Wincraft.

## Current limitations

This is an early desktop, not KDE feature parity. It does not yet implement
notification history, a StatusNotifier tray, Bluetooth UI, clipboard history,
per-display wallpaper selection, desktop file management, or graphical theme
editing. Network, sound, display management, file management and applications
are existing Linux tools. Window buttons currently show all windows on all
monitors. Hardware power management, suspend/resume, accessibility, locking,
GPU compatibility and installation require full ISO/VM and hardware testing.

## Implementation references

- [GTK4 Rust bindings](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/)
- [gtk4-layer-shell](https://docs.rs/gtk4-layer-shell/0.7.1/gtk4_layer_shell/)
- [labwc configuration](https://labwc.github.io/labwc-config.5.html)
