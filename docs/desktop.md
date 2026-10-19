# SegLinux Rust desktop

This is the first implementation of SegLinux's own desktop shell. The shell
is Rust with GTK4 Rust bindings; it is not a rewrite of the entire desktop
stack in Rust. GTK, labwc, greetd and the bundled applications retain their
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
  - **Start** (Super+Space), like Plasma's Kickoff: your name, search
    (names, descriptions and keywords; Enter starts the first result),
    *Favorites* (the dock's pins, in their order), *All applications* and
    the categories in use (Games, Internet, Multimedia, Graphics, Office,
    Development, ...), then Files, Settings, sign out, restart and shut
    down. Right-click an application (or press and hold) for its own
    actions (Brave's *New Incognito Window*), *Pin to dock* or *Unpin from
    dock*, and *Add to desktop*. The list follows installs: the desktop
    refreshes when applications are installed or removed, and ones
    installed since you first opened Start are marked *New* until you open
    them (the list of known applications is
    `~/.local/state/seglinux/known-apps`);
  - **Show desktop**: minimizes every window, or restores them;
  - **applications**: pinned launchers and running windows share one icon
    per application (matched by Wayland app id), with a line under running
    ones and a highlight for the active one. Click to start, focus or
    minimize; click again to cycle through several windows; middle-click
    for a new window; right-click for the application's own actions, *New
    window*, *Pin to dock* or *Unpin from dock*, *Move left* / *Move right*
    and *Close*. Drag a pinned icon onto another to reorder them. Pins are
    the `pinned` list of `shell.toml`, so they can be changed in Settings
    too;
  - **tray**, as Plasma's system tray: other applications' icons (Steam,
    Discord, Telegram, nm-applet... see below), network state (click for
    the connection editor, or nmtui), volume (click for the mixer, scroll
    to change, middle click to mute), battery on laptops, notifications,
    quick settings (a **Dark mode** switch, which applications that follow the
    system's light or dark preference follow too; sound, Bluetooth when the
    computer has it, display,
    lock, desktop settings, power) and the date and time (click for a
    calendar).
- **Notifications**: the desktop is the session's notification service
  (`org.freedesktop.Notifications`), so notifications from Brave, Steam,
  Discord, `notify-send` and every libnotify program appear, as in Plasma,
  as pop-ups at the bottom right above the dock: the application's icon and
  name, the title and text, and its buttons (*Show in folder*, *Reply*...).
  A click on one runs its default action; pointing at it keeps it on screen;
  critical ones (a nearly empty battery) stay until dismissed. The bell in
  the tray counts the unread ones and opens the history (newest first,
  *Clear all*) with **Do not disturb**, which keeps everything but critical
  notifications out of sight.
- **Applications in the tray**: the desktop is the session's system tray
  (StatusNotifierItem, which Qt, Electron and libappindicator/Ayatana
  programs use), so an application that hides in the tray when its window
  is closed, such as Steam or Discord, stays reachable. A click shows the
  application, a right click opens its menu (with submenus and check
  marks), a middle click runs its secondary action and scrolling is passed
  on. Icons asking for attention are highlighted.
- **Volume and brightness keys**: the change, then a level bar above the
  dock for a moment, as in Plasma. Volume keys raise (unmuting first),
  lower and mute the output; the microphone key mutes the input; the
  brightness keys change the screen's backlight in 5% steps, through
  elogind (no root needed, never fully dark).
- **From Plasma's desktop** (KDE plasma-desktop), in Rust:
  - **Emoji selector** (Super+., or *Emoji Selector* in Start): search,
    categories, recently used; the chosen emoji is copied, ready to paste.
  - **Calculator in Start's search**: `12*7`, `(3+4.5)/2`, `2^10`, `15%`,
    `sqrt 2`; Enter copies the answer.
  - **Badges on dock icons**: unread counts (Telegram, Discord,
    Thunderbird), download and copy progress (Brave, Dolphin) and requests
    for attention, which applications send with the Unity launcher API, as
    Plasma's task manager shows them.
  - **Trash on the desktop**, full or empty: double-click opens it (in
    Dolphin, which restores files), right-click empties it. Desktop icons
    have *Open*, *Rename…* and *Move to Trash*; the desktop, *New folder…*.
- **Steady**: if the desktop shell ever crashes, it starts again at once
  (dock, wallpaper and notifications come back; after five crashes in two
  minutes it stops trying). Notification pop-ups wait in the history while a
  fullscreen window (a game, a video) is in front; critical ones still show.
  At idle the shell wakes the processor about once every four seconds
  (it was 40 times a second before October 2026).
- **Terminal** (Super+Enter, kitty): each new window or tab starts with
  system information under SegLinux's logo (fastfetch); `fastfetch` shows it
  again. Shells started from a shell, scripts, and the terminal panels of
  Dolphin and Kate skip it. To turn it off:
  `mkdir -p ~/.config/seglinux && touch ~/.config/seglinux/no-fastfetch`
  (delete that file to turn it back on). The look is
  `/etc/xdg/fastfetch/config.jsonc`; a config of your own in
  `~/.config/fastfetch/` replaces it.
- **Sign-in screen** (`seglinux-greeter`, Rust and GTK 4, in place of
  SDDM): the clock over the wallpaper, the accounts, the password and the
  desktop to start, preselecting the SegLinux desktop and the last account
  (see [below](#sign-in-screen)); the Lite live session starts the desktop
  directly.
- labwc manages windows (decorations, snapping, Alt+Tab), like KWin under
  Plasma; without a usable GPU driver the session falls back to software
  rendering, then to wlroots' CPU renderer.
- Gaming: variable refresh rate and tearing for fullscreen games in labwc
  (`adaptiveSync`, `allowTearing`), and a separate **SegLinux Gaming Mode**
  session (Steam Big Picture in gamescope). See [gaming.md](gaming.md).

## Sign-in screen

SegLinux's sign-in screen is `seglinux-greeter`, a Rust (GTK 4) greeter for
[greetd](https://sr.ht/~kennylevinsen/greetd/), the small sign-in service
that replaced SDDM. greetd starts it on the seventh console as its unprivileged
`greeter` user, alone in a labwc compositor with no shortcuts and no menus
(`seglinux-greeter-session`). It shows:

- the clock and date over the wallpaper, on every monitor;
- the people's accounts (several: a row to choose from; *Other user* for
  one not listed), and the password field, which warns when Caps Lock is
  on and says when the password is wrong;
- the desktop to start, bottom left: **SegLinux**, Plasma (Wayland),
  SegLinux Gaming Mode, ... (the sessions of `/usr/share/wayland-sessions`);
- **Sleep**, **Restart** and **Shut down**, bottom right.

greetd checks the password with PAM, as SDDM did; the greeter only passes
it on. The account and desktop used last are preselected next time
(`/var/lib/seglinux-greeter/state.toml`; Setup writes the first one).
Ctrl+Alt+F2 still opens a text console.

Its look is set in `/etc/seglinux/greeter.toml` (as root):

```toml
wallpaper = "/usr/share/backgrounds/seglinux/seglinux-flow.jpg"
accent = "#e95420"          # sign-in button, focus rings, the round picture
clock_format = "%I:%M %p"   # 12-hour clock; "%H:%M" is 24-hour
```

**No automatic sign-in.** Installed systems always show the sign-in screen
and ask for the password; Setup no longer offers to skip it, and
`seglinux-upgrade` does not carry SDDM's automatic sign-in over when it
switches a system to this screen. (Only the live USB session signs in by
itself.) Anyone who still wants it can add greetd's `[initial_session]`
to `/etc/greetd/config.toml` by hand:

```toml
[initial_session]
command = "seglinux-session"
user = "yourname"
```

Try it without signing out: `seglinux-greeter --demo` opens it in a window
(the password `demo` "signs in" and prints the session it would start).

**Systems installed from an older ISO** keep SDDM until `sudo
seglinux-upgrade` has brought a SegLinux with the greeter (see
[updating.md](updating.md)): it then asks once whether to switch (OpenRC and
dinit), and the new sign-in screen appears at the next restart, keeping
SDDM's automatic sign-in if there was one. To switch by hand instead (OpenRC; for runit or dinit install `greetd-runit` or
`greetd-dinit` and switch the services with their own tools):

```sh
sudo seglinux-upgrade                       # brings seglinux-greeter (docs/updating.md)
sudo rc-update del sddm default
sudo pacman -Rdd sddm-openrc        # it conflicts with greetd-openrc
sudo pacman -S --needed greetd greetd-openrc
sudo cp /usr/share/seglinux/greeter/greetd.toml /etc/greetd/config.toml
sudo install -d -o greeter -g greeter /var/lib/seglinux-greeter
sudo rc-update add greetd default
```

and restart. (The switch removes `sddm-openrc`, which conflicts with
`greetd-openrc`.) To go back: `sudo rc-update del greetd default && sudo
pacman -S sddm-openrc && sudo rc-update add sddm default`.

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
| Super+. | Emoji selector |
| Super+Left / Right | Snap a window |
| Ctrl+Super+Left / Right | Previous / next workspace |
| Ctrl+Shift+Super+Left / Right | Move the window to the previous / next workspace |
| Alt+Tab | Window switcher |
| Volume keys | Volume up, down and mute, with the level on screen |
| Microphone mute key | Mute or unmute the microphone |
| Brightness keys | Screen brightness, with the level on screen |

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

**The whole system.** Measured on 15 October 2026 in a virtual machine (4 GB,
no GPU) with the live system, after sign-in, idle, a terminal open: 792 MB
in use (`free -m`). The largest parts: kitty 185 MB and its helper 31 MB,
labwc 154 MB (drawing with Mesa's software OpenGL, as without a GPU),
the desktop shell 60 MB, WirePlumber 33 MB, syslog-ng 28 MB, the polkit
agent 25 MB, NetworkManager 22 MB, xdg-desktop-portal and its helpers about
60 MB (the shell no longer starts them: applications start them when they
need them). Earlier versions of this page and of the website gave 59 MB as
if it were the system's memory: it was only the desktop's own processes,
measured on a virtual screen. On a real computer the GPU driver and the
applications you run add their share.

**The desktop's own processes**, measured on their own:

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
settings did not change it (55 MB before, 56 MB after, within noise), nor did the
notifications, the tray for other applications and the volume display
(re-measured on 14 October 2026: 56 MB). The number covers the desktop,
not the whole system: the kernel, PipeWire, the polkit agent, Xwayland (when
an X11 program runs) and services come on top. To check on your computer:

```sh
for p in labwc seglinux-shell dbus-daemon; do
  pid=$(pgrep -nx "$p") && grep -H '^Pss:' /proc/$pid/smaps_rollup
done
```

## Current limitations

This is a young desktop, not KDE feature parity. It does not yet implement
clipboard history, its own Bluetooth pairing screen (Quick settings opens
Blueman or Plasma's), per-display wallpaper selection, moving,
renaming or deleting desktop icons (use the Desktop folder in a file
manager), dragging applications from Start onto the dock, window previews,
per-monitor display arrangement
(use the Displays tool) or full application theme editing. Network, sound, display management, file management and applications
are existing Linux tools. Window buttons currently show all windows on all
monitors. Hardware power management, suspend/resume, accessibility, locking,
GPU compatibility and installation require full ISO/VM and hardware testing.
The sign-in screen shows initials rather than account pictures, has no
on-screen keyboard or layout switcher (it uses the layout chosen in Setup),
and does not unlock KDE Wallet at sign-in as SDDM did.

## Implementation references

- [GTK4 Rust bindings](https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/)
- [gtk4-layer-shell](https://docs.rs/gtk4-layer-shell/0.7.1/gtk4_layer_shell/)
- [labwc configuration](https://labwc.github.io/labwc-config.5.html)
