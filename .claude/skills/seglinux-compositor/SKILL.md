---
name: seglinux-compositor
description: Develop, test and screenshot seglinux-compositor, SegLinux's own Wayland compositor (crates/seglinux-compositor, Rust on Smithay 0.7). Use for any work on the compositor - new protocols, window management, XWayland, title bars, input, the udev/winit backends - and whenever the user asks to check, test or show (screenshots) that it works, or to make the SegLinux desktop run on it in place of labwc.
---

# seglinux-compositor

SegLinux's compositor, meant to replace labwc under the SegLinux desktop
(seglinux-shell: wallpaper, dock, Start menu, tray). Until it does all labwc
does it is a preview session: `data/desktop/seglinux-compositor-session`,
`seglinux-compositor.desktop` on the sign-in screen. labwc stays the default
until the user has tried it on real hardware and says so.

## Map

| File | What |
|---|---|
| `main.rs` | options (`--winit`, `--tty`, `--config`), event loop |
| `state.rs` | `State`, protocol handlers, window management (place, maximize, snap, minimize, focus) |
| `input.rs` | keys (config binds first), pointer, title bar clicks |
| `config.rs` | `~/.config/seglinux/compositor.toml`; defaults mirror `data/desktop/labwc/rc.xml` (a test checks) |
| `render.rs` | what is drawn per output, front to back: pointer, overlay/top layers, windows with bars, bottom/background |
| `deco.rs` | server-side title bars (fontdue text, buttons) |
| `foreign_toplevel.rs` | the dock's window list (wlr-foreign-toplevel v3, with output_enter) |
| `focus.rs` | keyboard focus: Wayland surface or X11 window |
| `xwayland.rs` | XWayland, X11 window management |
| `udev.rs` / `winit.rs` | real screens (libseat, DRM, libinput) / a window for testing |
| `protocols.rs` | relative-pointer + pointer-constraints (games), xdg-activation, fractional scale, xdg-foreign |
| `examples/windows.rs` | a dock stand-in: list, minimize, restore, maximize, activate, close |

## Build

Needs (Ubuntu names): `libxkbcommon-dev libinput-dev libseat-dev libudev-dev
libgbm-dev libdrm-dev libegl-dev libgles-dev`; CI installs them in ci.yml.
Before every push:

```sh
cargo fmt --all && cargo clippy -p seglinux-compositor --all-targets -- -D warnings \
  && cargo test -p seglinux-compositor && cargo fmt --all --check
```

Run `cargo fmt` again after `cargo clippy --fix`. CI's clippy can be newer than
the local one (it flagged `chunks_exact` once): read CI's log, apply its hint.

## Test with screenshots (no GPU needed)

```sh
apt-get install -y weston xwayland foot waybar swaybg fuzzel xterm x11-apps
export XDG_RUNTIME_DIR=/tmp/xdg; mkdir -p -m700 $XDG_RUNTIME_DIR
weston --debug --backend=headless --renderer=gl --socket=wl-host --width=1400 --height=900 &
# compositor.toml autostart: swaybg (wallpaper), waybar with wlr/taskbar
# (stands in for the dock: same layer-shell + foreign-toplevel), foot, xterm
WAYLAND_DISPLAY=wl-host XDG_CONFIG_HOME=/tmp/cfg dbus-run-session -- \
  target/debug/seglinux-compositor --winit &
WAYLAND_DISPLAY=wl-host weston-screenshooter   # wayland-screenshot-*.png
WAYLAND_DISPLAY=wayland-1 target/debug/examples/windows minimize foot
```

- Always use `timeout` on screenshooter and the example; a hang means Weston
  stopped repainting: restart Weston.
- Screen sharing: `wf-recorder -f rec.mp4` (copy_with_damage, as
  xdg-desktop-portal-wlr): a still screen gives one frame. Its video is upside
  down under `--winit` only (that output reports Flipped180); real screens
  are Normal.
- `wayland-info` (wayland-utils) lists the globals: check a new protocol shows.
- Headless Weston gives no pointer: input paths (constraints, grabs) can only
  be checked for protocol errors here (`weston-constraints` runs clean).
- Weston gone after a session restart (stale `wl-host` socket, winit says
  NoCompositor): `rm /tmp/xdg/wl-host*` and start it again.
- Waybar needs a D-Bus session (`dbus-run-session`), as the real session has.
- `pkill -x seglinux-compos` (process names are cut to 15 characters);
  never `pkill -f seglinux-compositor`, it kills your own shell.
- The real dock (seglinux-shell) needs gtk4-layer-shell, which may not be
  installable; say so instead of claiming the dock was tested.
- Keyboard focus: run a client with `WAYLAND_DEBUG=1` and look for
  `wl_keyboard.enter`. Input cannot be injected: clicks are covered by unit
  tests (`deco::hit`), say they were not clicked.
- Look at every screenshot before reporting it, and send them to the user.

## Rules learned

- A window can be Wayland (`toplevel()`) or X11 (`x11_surface()`): never
  `unwrap()` either; find windows by `WaylandFocus::wl_surface()`.
- XWayland's client carries `XWaylandClientData`, not `ClientState`.
- The autostart runs after XWayland is ready, so the desktop gets `$DISPLAY`.
- Bind `layer_map_for_output` results to a variable before `if let` when the
  body needs `&mut self` (edition 2024 temporaries).
- New user-visible work: update `docs/compositor.md` (What works / Not yet)
  and `CHANGELOG.md`; one PR per step, merged only when the user asks.
