//! Keeps the desktop running. The process the session starts does not draw
//! anything: it runs the desktop shell and starts it again if it crashes, so
//! a fault in the dock never leaves the session without a dock, wallpaper
//! or notifications (the way Plasma's session restarts plasmashell).
use gtk::glib;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Set for the shell the supervisor runs.
pub const CHILD: &str = "SEGLINUX_SHELL_SUPERVISED";
/// More crashes than this within `WINDOW` and the supervisor gives up
/// rather than restart a desktop that cannot run.
const RESTARTS: usize = 5;
const WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// It quit on purpose, or the session is over.
    Stop,
    Restart,
    GiveUp,
}

/// What to do once the shell has ended.
pub fn next(clean_exit: bool, compositor_alive: bool, recent_crashes: usize) -> Next {
    if clean_exit || !compositor_alive {
        Next::Stop
    } else if recent_crashes >= RESTARTS {
        Next::GiveUp
    } else {
        Next::Restart
    }
}

/// Whether the Wayland compositor still accepts clients: when it has gone
/// (signing out), the shell's end is the session's.
fn compositor_alive() -> bool {
    let Some(display) = std::env::var_os("WAYLAND_DISPLAY") else {
        return false;
    };
    let display = PathBuf::from(display);
    let path = if display.is_absolute() {
        display
    } else {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) => PathBuf::from(dir).join(display),
            None => return false,
        }
    };
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// The shell's program, also after an update replaced it (the new one).
fn program() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|path| {
            let text = path.to_string_lossy();
            match text.strip_suffix(" (deleted)") {
                Some(original) => PathBuf::from(original),
                None => path,
            }
        })
        .unwrap_or_else(|| PathBuf::from("seglinux-shell"))
}

pub fn run(args: &[String]) -> glib::ExitCode {
    let program = program();
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        let status = match Command::new(&program).args(args).env(CHILD, "1").status() {
            Ok(status) => status,
            Err(error) => {
                eprintln!(
                    "seglinux-shell: cannot start {}: {error}",
                    program.display()
                );
                return glib::ExitCode::FAILURE;
            }
        };
        crashes.retain(|t| t.elapsed() < WINDOW);
        match next(status.success(), compositor_alive(), crashes.len()) {
            Next::Stop if status.success() => return glib::ExitCode::SUCCESS,
            Next::Stop => return glib::ExitCode::FAILURE,
            Next::GiveUp => {
                eprintln!(
                    "seglinux-shell: the desktop stopped {RESTARTS} times in {} seconds ({status}); not starting it again",
                    WINDOW.as_secs()
                );
                return glib::ExitCode::FAILURE;
            }
            Next::Restart => {
                eprintln!("seglinux-shell: the desktop stopped ({status}); starting it again");
                crashes.push(Instant::now());
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_only_what_crashed_while_the_session_runs() {
        assert_eq!(next(false, true, 0), Next::Restart);
        assert_eq!(next(false, true, RESTARTS - 1), Next::Restart);
        assert_eq!(next(false, true, RESTARTS), Next::GiveUp);
        assert_eq!(next(true, true, 0), Next::Stop, "quit on purpose");
        assert_eq!(next(false, false, 0), Next::Stop, "the session ended");
    }
}
