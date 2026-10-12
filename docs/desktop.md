# Wincraft Rust desktop

This is the first implementation of Wincraft's own desktop shell. The shell
is Rust with GTK4 Rust bindings; it is not a rewrite of the entire desktop
stack in Rust. GTK, labwc, SDDM and the bundled applications retain their
upstream implementations. labwc handles composition, decorations, snapping,
Alt+Tab and workspaces. Plasma remains an alternative session.

## Implemented

Laid out like a Plasma desktop with a floating panel:

- **Wallpaper**: `wincraft-flow.jpg`, drawn from scratch by
  `data/artwork/make-wallpaper.py`; the earlier dark wallpaper stays installed.
- **Desktop icons**: the files and folders of `~/Desktop` on the first
  monitor, folders first; double-click opens them with their default
  application (desktop entries start their application) and the icons follow
  changes to the folder. Right-click the desktop to open a terminal there, the
  Desktop folder, or the desktop settings.
- **Floating dock**: centred at the bottom, rounded and translucent, only as
  wide as its contents, on every monitor:
  - **Start** (Super+Space): your name, search across installed
    applications, Files, Settings, sign out, restart and shut down;
  - **Show desktop**: minimizes every window, or restores them;
  - **applications**: pinned launchers and running windows share one icon
    per application (matched by Wayland app id), with a line under running
    ones and a highlight for the active one. Click to start, focus or
    minimize; click again to cycle through several windows; middle-click
    for a new window; right-click for *New window* and *Close*;
  - **tray**: network state (click for the connection editor, or nmtui),
    volume (click for the mixer, scroll to change), battery on laptops,
    quick settings, and the date and time (click for a calendar).
- **Default session**: Wincraft Setup makes the Wincraft desktop the
  preselected session on SDDM's sign-in screen (Plasma remains selectable);
  the Lite live session starts it directly.
- labwc manages windows (decorations, snapping, Alt+Tab), like KWin under
  Plasma; without a usable GPU driver the session falls back to software
  rendering, then to wlroots' CPU renderer.
- Gaming: variable refresh rate and tearing for fullscreen games in labwc
  (`adaptiveSync`, `allowTearing`), and a separate **Wincraft Gaming Mode**
  session (Steam Big Picture in gamescope). See [gaming.md](gaming.md).

## Try it without switching desktop

```sh
cargo run -p wincraft-shell -- --preview --wallpaper "$PWD/data/artwork/wincraft-flow.jpg"
```

Preview mode intentionally has no running-window integration and disables
locking and power-off. Its launcher and shortcuts open real installed apps.
Use the packaged **Wincraft** login session for the Wayland desktop.
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

Run `wincraft-shell --settings`, open Settings from Start, or right-click the
desktop. Choose an existing image by absolute path, a 12-hour or 24-hour
clock, and whether the desktop shows icons. Changes apply next session. For
the pinned applications and other clock formats, copy
`/usr/share/wincraft/shell.example.toml` to `~/.config/wincraft/shell.toml`:

```toml
wallpaper = "/usr/share/backgrounds/wincraft/wincraft-flow.jpg"
clock_format = "%x  %H:%M"        # %x: the date in your language's format
pinned = ["org.kde.dolphin", "firefox", "wincraft-terminal", "steam", "net.lutris.Lutris"]
desktop_icons = true
```
`XDG_CONFIG_HOME` is respected. Invalid configurations log a warning and use
defaults. Settings are saved atomically.

The compositor uses `/usr/share/wincraft/labwc`. These distribution defaults
are deliberately independent of any existing personal labwc configuration.
The session does not run arbitrary XDG autostart entries yet; its startup
script explicitly launches the components shipped with Wincraft.

## Current limitations

This is a young desktop, not KDE feature parity. It does not yet implement
notification history, a StatusNotifier tray for other applications' icons,
Bluetooth UI, clipboard history, per-display wallpaper selection, moving,
renaming or deleting desktop icons (use the Desktop folder in a file
manager), pinning from the dock, or graphical theme editing. Network, sound, display management, file management and applications
are existing Linux tools. Window buttons currently show all windows on all
monitors. Hardware power management, suspend/resume, accessibility, locking,
GPU compatibility and installation require full ISO/VM and hardware testing.

## Implementation references

- [GTK4 Rust bindings](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/)
- [gtk4-layer-shell](https://docs.rs/gtk4-layer-shell/0.7.1/gtk4_layer_shell/)
- [labwc configuration](https://labwc.github.io/labwc-config.5.html)
