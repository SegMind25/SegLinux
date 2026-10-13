# SegLinux Rust desktop

This is the first implementation of SegLinux's own desktop shell. The shell
is Rust with GTK4 Rust bindings; it is not a rewrite of the entire desktop
stack in Rust. GTK, labwc, SDDM and the bundled applications retain their
upstream implementations. labwc handles composition, decorations, snapping,
Alt+Tab and workspaces. Plasma remains an alternative session.

## Implemented

Laid out like a Plasma desktop with a floating panel:

- **Wallpaper**: `seglinux-dark.jpg`, the SegLinux S drawn in ribbons of
  light by `data/artwork/make-brand.py`; the calmer `seglinux-flow.jpg`
  (`data/artwork/make-wallpaper.py`) is installed next to it.
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
- **Default session**: SegLinux Setup makes the SegLinux desktop the
  preselected session on SDDM's sign-in screen (Plasma remains selectable);
  the Lite live session starts it directly.
- labwc manages windows (decorations, snapping, Alt+Tab), like KWin under
  Plasma; without a usable GPU driver the session falls back to software
  rendering, then to wlroots' CPU renderer.
- Gaming: variable refresh rate and tearing for fullscreen games in labwc
  (`adaptiveSync`, `allowTearing`), and a separate **SegLinux Gaming Mode**
  session (Steam Big Picture in gamescope). See [gaming.md](gaming.md).

## Try it without switching desktop

```sh
cargo run -p seglinux-shell -- --preview --wallpaper "$PWD/data/artwork/seglinux-dark.jpg"
```

Preview mode intentionally has no running-window integration and disables
locking and power-off. Its launcher and shortcuts open real installed apps.
Use the packaged **SegLinux** login session for the Wayland desktop.
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

Run `seglinux-shell --settings`, open Settings from Start, or right-click the
desktop. The Rust settings window has four pages:

- **Appearance:** choose an image with the file picker, cover/fit/stretch layout,
  dark/light shell colors, a validated hex accent, text size and animations.
- **Desktop & dock:** desktop icons and their size, dock size and top/bottom
  placement, ordered pinned application IDs, and a custom clock format with
  12-hour and 24-hour presets.
- **Gaming:** Wine/Proton/umu runner, Windows environment, installer isolation,
  GameMode, DXVK, VKD3D-Proton, esync/fsync, performance overlay, debug output,
  and gamescope fullscreen, resolution, frame-rate and custom arguments.
- **System:** opens installed network, audio, display, Bluetooth, printing,
  Steam and Lutris tools. Missing tools are disabled and identified; these
  controls do not install packages or pretend to replace their settings.

Save appearance and desktop preferences together. **Shell changes apply after
signing out and back in.** They affect the SegLinux shell, not GTK/KDE themes
in other applications. Gaming preferences apply on the next `segrun` launch;
Steam and Lutris retain their own settings. Existing per-game overrides and
custom environment variables are preserved when gaming defaults are saved.

These expanded controls require a build containing the settings update;
previously published ISO images are not modified by updating the website.

Preferences live in `~/.config/seglinux/shell.toml` (`XDG_CONFIG_HOME` is
respected). The example at `/usr/share/seglinux/shell.example.toml` includes:

```toml
wallpaper = "/usr/share/backgrounds/seglinux/seglinux-dark.jpg"
clock_format = "%x  %H:%M"
pinned = ["org.kde.dolphin", "firefox", "seglinux-terminal", "steam"]
desktop_icons = true
theme = "dark"
accent = "#3daee9"
font_size = 13
animations = true
wallpaper_fit = "cover"
dock_position = "bottom"
dock_icon_size = 26
desktop_icon_size = 48
```

Existing configuration files keep their defaults for new options. Invalid
configurations log a warning and use defaults. Saves are atomic; invalid UI
input is rejected before overwriting preferences. An unchanged missing default
wallpaper does not prevent saving unrelated settings.

The compositor uses `/usr/share/seglinux/labwc`. These distribution defaults
are deliberately independent of any existing personal labwc configuration.
The session does not run arbitrary XDG autostart entries yet; its startup
script explicitly launches the components shipped with SegLinux.

## Current limitations

This is a young desktop, not KDE feature parity. It does not yet implement
notification history, a StatusNotifier tray for other applications' icons,
Bluetooth UI, clipboard history, per-display wallpaper selection, moving,
renaming or deleting desktop icons (use the Desktop folder in a file
manager), pinning directly from the dock, or full application theme editing. Network, sound, display management, file management and applications
are existing Linux tools. Window buttons currently show all windows on all
monitors. Hardware power management, suspend/resume, accessibility, locking,
GPU compatibility and installation require full ISO/VM and hardware testing.

## Implementation references

- [GTK4 Rust bindings](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/)
- [gtk4-layer-shell](https://docs.rs/gtk4-layer-shell/0.7.1/gtk4_layer_shell/)
- [labwc configuration](https://labwc.github.io/labwc-config.5.html)
