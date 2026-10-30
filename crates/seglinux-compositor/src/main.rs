//! seglinux-compositor: SegLinux's own Wayland compositor, written in Rust
//! on Smithay. It runs the SegLinux desktop (seglinux-shell) as labwc does,
//! with its keys and settings in ~/.config/seglinux/compositor.toml.
//!
//! Based on Smithay's smallvil and anvil examples (MIT licence, Copyright
//! (c) 2017 Victor Berger and Smithay contributors).

mod config;
mod cursor;
mod deco;
mod foreign_toplevel;
mod grabs;
mod input;
mod pointer;
mod render;
mod state;
mod udev;
mod winit;

use std::path::PathBuf;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

const HELP: &str = "\
Usage: seglinux-compositor [--winit | --tty] [--config FILE]

SegLinux's Wayland compositor. Started from a text console (or the sign-in
screen) it uses the computer's screens; inside another desktop it opens a
window (--winit forces that, --tty the other).

  --config FILE   settings (default ~/.config/seglinux/compositor.toml)
  --version       print the version

Keys: Super+Return terminal, Super+Space Start, Alt+Tab next window,
Alt+F4 close, Super+Up maximize, Super+Left/Right snap, Super+drag move,
Ctrl+Alt+Backspace quit. RUST_LOG=debug for details.
";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut nested =
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let mut config_path = state_config_default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--winit" => nested = true,
            "--tty" => nested = false,
            "--config" => config_path = args.next().map(PathBuf::from),
            "--version" => {
                println!("seglinux-compositor {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other => {
                eprintln!("seglinux-compositor: unknown option `{other}`\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut event_loop: EventLoop<'static, state::State> = EventLoop::try_new()?;
    let display: Display<state::State> = Display::new()?;
    let mut state = state::State::new(
        display,
        event_loop.handle(),
        event_loop.get_signal(),
        if nested { "winit" } else { "seat0" },
        config_path,
    );

    if nested {
        winit::init(&mut event_loop, &mut state)?;
    } else {
        udev::init(&mut event_loop, &mut state)?;
    }

    // Applications started from here (and the autostart) find us.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
    }
    tracing::info!("listening on {}", state.socket_name.to_string_lossy());
    state.autostart();

    event_loop.run(None, &mut state, |state| {
        state.space.refresh();
        state.refresh_foreign_toplevels();
        state.popups.cleanup();
        let _ = state.display_handle.flush_clients();
    })?;
    Ok(())
}

fn state_config_default() -> Option<PathBuf> {
    config::Config::default_path()
}
