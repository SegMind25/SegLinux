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
| Super+I | Settings |
| Super+Left / Right | Snap a window |
| Ctrl+Super+Left / Right | Previous / next workspace |
| Ctrl+Shift+Super+Left / Right | Move the window to the previous / next workspace |
| Alt+Tab | Window switcher |
| Volume keys | PipeWire volume and mute |

## Settings

Press Super+I, open Settings from Start, or right-click the desktop
(`seglinux-shell --settings [appearance|desktop|windows|gaming|system]`).
Pages are listed on the left; **Apply** saves every page at once and the
desktop uses the new settings immediately, without signing out.
**Restore page defaults** shows the current page's defaults, saved on Apply.

- **Appearance:** dark or light theme; accent color (ten swatches or any hex
  color); text size; animations; the wallpaper (thumbnails of the installed
  ones, or any image) and how it fills the screen; the dock's opacity.
- **Desktop & dock:** desktop icons and their size; the dock at the top or
  bottom, its icon size, the Show desktop button and the battery percentage;
  the clock (presets or any strftime format, with a live preview); pinned
  applications: pin any installed application, reorder, unpin.
- **Windows & input:** focus follows the mouse, raise on focus, the gap
  around snapped windows, title bar roundness, 1 to 12 workspaces; keyboard
  layouts (several, with Alt+Shift, Ctrl+Shift or Caps Lock to switch), key
  repeat rate and delay, Num Lock; pointer speed and acceleration, natural
  scrolling, left-handed buttons, pointer size; touchpad tap-to-click,
  natural scrolling and disable-while-typing.
- **Gaming:** see [gaming.md](gaming.md).
- **System:** the computer's system, kernel, processor, memory and graphics;
  the installed network, sound, display, Bluetooth, printer, Steam and Lutris
  tools (missing ones are disabled); the settings folder.

Apply checks every page before writing anything: an invalid value (such as
`#12345` for a color, or an unknown keyboard layout syntax) is reported
with its page and nothing is saved. Only files whose content changed are
written, atomically.

### Where settings are stored

| File | What |
| --- | --- |
| `~/.config/seglinux/shell.toml` | Everything on the Appearance, Desktop & dock and Windows & input pages, the overlay and Gaming Mode. The desktop watches it and applies changes; a file being edited by hand that does not parse is ignored until it does. |
| `~/.config/seglinux/labwc/` | The compositor's configuration. Its files link to the defaults in `/usr/share/seglinux/labwc` until a window or input setting changes; then `rc.xml` and `environment` are generated and labwc reloads them. |
| `~/.config/seglinux/segrun.toml` | How Windows programs run (see gaming.md). |
| `~/.config/MangoHud/MangoHud.conf` | The performance overlay's position, detail and frame rate limit. |

Generated files start with *Managed by SegLinux Settings*. Delete that line
to keep your own version: Settings then leaves the file alone and says so.
`XDG_CONFIG_HOME` is respected. `/usr/share/seglinux/shell.example.toml`
lists every setting with its default.

The keyboard layout chosen during Setup is used until another one is set in
Settings. These are settings of the SegLinux session; the Plasma session
keeps its own.

## Resource use

The desktop's idle footprint, re-measured on 13 October 2026 with release
builds: labwc on a 1920×1080 headless output (CPU renderer), the shell
started as the session's autostart does (`GSK_RENDERER=cairo`), and D-Bus,
25 seconds after start, then CPU over 60 seconds of idle:

| Process | RSS | PSS |
| --- | --- | --- |
| labwc | 30 MB | 19 MB |
| seglinux-shell | 46 MB | 35 MB |
| dbus-daemon and dbus-run-session | 7 MB | 2 MB |
| **Total** | **83 MB** | **56 MB** |

CPU: 0.2% of one core. PSS counts each shared library once across the
processes, which is the fair way to add them up; RSS counts GTK and the C
library in every process. The 59 MB quoted on the website was measured the
same way (PSS, on a virtual screen) when the shell moved to GTK's CPU
renderer: shell 39.5 MB, labwc 16.4 MB, D-Bus 3 MB, 0.25% CPU. The live
settings did not change it (55 MB before, 56 MB after, within noise). The number covers the desktop,
not the whole system: the kernel, PipeWire, the polkit agent, Xwayland (when
an X11 program runs) and services come on top. To check on your computer:

```sh
for p in labwc seglinux-shell dbus-daemon; do
  pid=$(pgrep -nx "$p") && grep -H '^Pss:' /proc/$pid/smaps_rollup
done
```

## Current limitations

This is a young desktop, not KDE feature parity. It does not yet implement
notification history, a StatusNotifier tray for other applications' icons,
Bluetooth UI, clipboard history, per-display wallpaper selection, moving,
renaming or deleting desktop icons (use the Desktop folder in a file
manager), pinning directly from the dock, per-monitor display arrangement
(use the Displays tool) or full application theme editing. Network, sound, display management, file management and applications
are existing Linux tools. Window buttons currently show all windows on all
monitors. Hardware power management, suspend/resume, accessibility, locking,
GPU compatibility and installation require full ISO/VM and hardware testing.

## Implementation references

- [GTK4 Rust bindings](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/)
- [gtk4-layer-shell](https://docs.rs/gtk4-layer-shell/0.7.1/gtk4_layer_shell/)
- [labwc configuration](https://labwc.github.io/labwc-config.5.html)
