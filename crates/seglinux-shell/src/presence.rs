//! Discord Rich Presence for the whole desktop: the application in use
//! shows on the Discord profile ("Firefox, for 12 minutes"; a game by its
//! name), as games do on Windows. Discord shows it under the name of a
//! Discord application, made once at discord.com/developers/applications
//! and named as it should read ("SegLinux"); its ID goes in shell.toml:
//!
//! ```toml
//! [discord]
//! client_id = "123456789012345678"
//! show_titles = false   # window titles (web pages, documents): private
//! ```
//!
//! Discord, Vesktop and WebCord (native, Flatpak or Snap) are found on
//! their local socket; nothing goes over the network from here.

use std::{
    cell::RefCell,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{apps, config::Discord, tasks::Task};

const LOGO: &str = "https://raw.githubusercontent.com/SegMind25/SegLinux/main/data/grub/seglinux/icons/seglinux.png";
/// Discord takes 5 updates per 20 seconds.
const MIN_GAP: Duration = Duration::from_secs(4);
/// Discord not running: looked for again this often.
const RETRY: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Activity {
    client_id: String,
    app: String,
    title: Option<String>,
    /// Unix seconds: when this application came to the front.
    since: u64,
}

static SENDER: OnceLock<Sender<Option<Activity>>> = OnceLock::new();

thread_local! {
    /// The application shown and since when (a new title keeps the time).
    static SHOWN: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// The windows changed: show the active one's application on Discord.
pub fn update(tasks: &[Task], discord: &Discord) {
    let client_id = discord.client_id.trim();
    if client_id.is_empty() {
        send(None);
        return;
    }
    let Some(task) = tasks.iter().find(|t| t.active) else {
        // The desktop itself.
        SHOWN.with(|s| s.borrow_mut().take());
        send(None);
        return;
    };
    let id = task.app_id.to_lowercase();
    if ["discord", "vesktop", "webcord"]
        .iter()
        .any(|d| id.contains(d))
    {
        // Talking about it: what was shown stays.
        return;
    }
    let app = app_name(task);
    let since = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        match &*s {
            Some((shown, since)) if *shown == app => *since,
            _ => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                *s = Some((app.clone(), now));
                now
            }
        }
    });
    let title = (discord.show_titles && !task.title.is_empty() && task.title != app)
        .then(|| task.title.clone());
    send(Some(Activity {
        client_id: client_id.to_string(),
        app,
        title,
        since,
    }));
}

/// A game by its title (Steam games, fullscreen windows), any other
/// application by its menu name.
fn app_name(task: &Task) -> String {
    if (task.fullscreen || task.app_id.starts_with("steam_app_")) && !task.title.is_empty() {
        return task.title.clone();
    }
    apps::lookup(&task.app_id)
        .map(|a| gtk::prelude::AppInfoExt::name(&a).to_string())
        .unwrap_or_else(|| task.app_id.clone())
}

fn send(activity: Option<Activity>) {
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run(rx));
        tx
    });
    let _ = sender.send(activity);
}

/// The connection to Discord, in its own thread: the newest activity wins.
fn run(rx: Receiver<Option<Activity>>) {
    let mut want: Option<Activity> = None;
    let mut conn: Option<(UnixStream, String)> = None;
    // What Discord shows now (None: unknown).
    let mut shown: Option<Option<Activity>> = None;
    let mut last = Instant::now() - MIN_GAP;
    loop {
        match rx.recv_timeout(RETRY) {
            Ok(a) => want = a,
            // Sent again now and then: a restarted Discord shows it too.
            Err(RecvTimeoutError::Timeout) => shown = None,
            Err(RecvTimeoutError::Disconnected) => return,
        }
        std::thread::sleep(MIN_GAP.saturating_sub(last.elapsed()));
        while let Ok(a) = rx.try_recv() {
            want = a;
        }
        if conn.is_some() && shown.as_ref() == Some(&want) {
            continue;
        }
        let client_id = match (&want, &conn) {
            (Some(a), _) => a.client_id.clone(),
            // Nothing to show and not connected: nothing to clear.
            (None, None) => continue,
            (None, Some((_, id))) => id.clone(),
        };
        if conn.as_ref().is_none_or(|(_, id)| *id != client_id) {
            conn = connect(&client_id).map(|s| (s, client_id));
            shown = None;
        }
        let Some((stream, _)) = conn.as_mut() else {
            continue;
        };
        last = Instant::now();
        match set_activity(stream, want.as_ref()) {
            Ok(()) => shown = Some(want.clone()),
            Err(e) => {
                eprintln!("seglinux-shell: Discord: {e}");
                conn = None;
            }
        }
    }
}

/// Where Discord listens: native, Flatpak, Snap, Vesktop.
fn sockets() -> Vec<PathBuf> {
    let run = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dirs = [
        run.clone(),
        run.join("app/com.discordapp.Discord"),
        run.join("app/dev.vencord.Vesktop"),
        run.join(".flatpak/dev.vencord.Vesktop/xdg-run"),
        run.join("snap.discord"),
        PathBuf::from("/tmp"),
    ];
    dirs.iter()
        .flat_map(|d| (0..10).map(move |i| d.join(format!("discord-ipc-{i}"))))
        .collect()
}

fn connect(client_id: &str) -> Option<UnixStream> {
    sockets().into_iter().find_map(|path| {
        let mut s = UnixStream::connect(path).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let hello = serde_json::json!({ "v": 1, "client_id": client_id });
        frame(&mut s, 0, &hello).ok()?;
        match reply(&mut s) {
            Ok(()) => Some(s),
            Err(e) => {
                eprintln!("seglinux-shell: Discord: {e}");
                None
            }
        }
    })
}

fn set_activity(s: &mut UnixStream, activity: Option<&Activity>) -> io::Result<()> {
    let activity = activity.map(|a| {
        let mut v = serde_json::json!({
            "details": clip(&a.app),
            "timestamps": { "start": a.since },
            "assets": { "large_image": LOGO, "large_text": "SegLinux" },
        });
        if let Some(t) = &a.title {
            v["state"] = clip(t).into();
        }
        v
    });
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let cmd = serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": std::process::id(), "activity": activity },
        "nonce": nonce.to_string(),
    });
    frame(s, 1, &cmd)?;
    reply(s)
}

/// Discord's limits: 2 to 128 characters.
fn clip(text: &str) -> String {
    let mut t: String = text.chars().take(128).collect();
    if t.chars().count() < 2 {
        t.push(' ');
    }
    t
}

fn frame(s: &mut UnixStream, op: u32, body: &serde_json::Value) -> io::Result<()> {
    let body = body.to_string();
    let mut buf = Vec::with_capacity(8 + body.len());
    buf.extend(op.to_le_bytes());
    buf.extend((body.len() as u32).to_le_bytes());
    buf.extend(body.as_bytes());
    s.write_all(&buf)
}

/// Reads Discord's answer; a refusal (an unknown application ID) is an error.
fn reply(s: &mut UnixStream) -> io::Result<()> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    if len > 1 << 20 {
        return Err(io::Error::other("answer too long"));
    }
    let mut body = vec![0; len];
    s.read_exact(&mut body)?;
    let text = String::from_utf8_lossy(&body);
    // 2: closed (wrong ID); an "ERROR" event: the activity was refused.
    if op == 2 || text.contains(r#""evt":"ERROR""#) {
        return Err(io::Error::other(text.into_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_limits() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        frame(&mut a, 1, &serde_json::json!({ "evt": "READY" })).unwrap();
        assert!(reply(&mut b).is_ok());
        frame(&mut a, 2, &serde_json::json!({ "code": 4000 })).unwrap();
        assert!(reply(&mut b).is_err());
        assert_eq!(clip("X"), "X ");
        assert_eq!(clip(&"a".repeat(300)).len(), 128);
    }
}
