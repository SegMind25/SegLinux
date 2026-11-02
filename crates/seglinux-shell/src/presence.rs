//! Discord Rich Presence for the whole desktop: the application in use
//! shows on the Discord profile ("Firefox, for 12 minutes"; a game by its
//! name), as games do on Windows: on by default, shown under SegLinux's
//! Discord application. In shell.toml (Settings, Gaming):
//!
//! ```toml
//! [discord]
//! activity = true
//! client_id = ""        # another Discord application's ID to show it under
//! show_titles = false   # window titles (web pages, documents): private
//! ```
//!
//! Discord, Vesktop and WebCord (native, Flatpak or Snap) are found on
//! their local socket; nothing goes over the network from here.

use std::{
    cell::RefCell,
    collections::HashMap,
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

/// SegLinux's Discord application: what Discord shows the activity under.
const SEGLINUX_APP_ID: &str = "1290295496077738019";
/// Application icons as PNG, by name (dashboard-icons: Brave, Firefox,
/// Spotify, Steam, VS Code and many more).
const ICONS: &str = "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/";
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
    /// Discord's verb: 0 Playing, 2 Listening, 3 Watching.
    kind: u8,
    /// Pictures that may show the application, the likeliest first.
    icons: Vec<String>,
    /// A game: shown as that game, under its own Discord application.
    game: Option<Game>,
}

/// What may identify a game in Discord's list of games.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Game {
    steam: Option<String>,
    /// Its title and program name, lowercase.
    names: Vec<String>,
}

static SENDER: OnceLock<Sender<Option<Activity>>> = OnceLock::new();

thread_local! {
    /// The application shown and since when (a new title keeps the time).
    static SHOWN: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// The windows changed: show the active one's application on Discord.
pub fn update(tasks: &[Task], discord: &Discord) {
    let client_id = match discord.client_id.trim() {
        "" => SEGLINUX_APP_ID,
        id => id,
    };
    if !discord.activity || client_id.is_empty() {
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
    let (app, kind, icons, game) = describe(task);
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
        kind,
        icons,
        game,
    }));
}

/// The application's name, Discord's verb for it, and pictures of it. A
/// game goes by its title (Steam games, fullscreen windows) with its Steam
/// artwork; any other application by its menu name and icon.
fn describe(task: &Task) -> (String, u8, Vec<String>, Option<Game>) {
    use gtk::prelude::{AppInfoExt, IconExt};
    if let Some(id) = task.app_id.strip_prefix("steam_app_")
        && !task.title.is_empty()
    {
        let art = format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/header.jpg");
        let game = Game {
            steam: Some(id.to_string()),
            names: vec![task.title.to_lowercase()],
        };
        return (task.title.clone(), 0, vec![art], Some(game));
    }
    if task.fullscreen && !task.title.is_empty() {
        let game = Game {
            steam: None,
            names: vec![task.title.to_lowercase(), task.app_id.to_lowercase()],
        };
        return (task.title.clone(), 0, Vec::new(), Some(game));
    }
    let info = apps::lookup(&task.app_id);
    let name = info
        .as_ref()
        .map(|a| a.name().to_string())
        .unwrap_or_else(|| task.app_id.clone());
    let categories = info
        .as_ref()
        .and_then(|a| a.categories())
        .map(|c| c.to_string())
        .unwrap_or_default();
    let has = |c: &str| categories.split(';').any(|x| x == c);
    let kind = if has("Game") {
        0
    } else if has("Video") || has("TV") {
        3
    } else if has("Audio") || has("Music") {
        2
    } else {
        0
    };
    let icon = info
        .as_ref()
        .and_then(|a| a.icon())
        .and_then(|i| IconExt::to_string(&i))
        .map(|s| s.to_string())
        .unwrap_or_default();
    let icons = icon_names([icon.as_str(), &task.app_id, &name])
        .into_iter()
        .map(|n| format!("{ICONS}{n}.png"))
        .collect();
    let game = has("Game").then(|| Game {
        steam: None,
        names: vec![name.to_lowercase(), task.app_id.to_lowercase()],
    });
    (name, kind, icons, game)
}

/// Names an icon collection may know an application by: "brave-browser",
/// "org.mozilla.firefox", "Visual Studio Code" give brave, firefox and
/// visual-studio-code.
fn icon_names<'a>(from: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    const GENERIC: &[&str] = &["client", "browser", "desktop", "app", "stable", "bin"];
    let mut names = Vec::new();
    for raw in from {
        // A path (an icon file): its name.
        let raw = raw.rsplit('/').next().unwrap_or(raw);
        let raw = raw
            .split('.')
            .filter(|p| !["png", "svg", "desktop"].contains(p));
        // Reverse-DNS ids: the last part that is not a generic word.
        let part = raw
            .rev()
            .map(str::to_lowercase)
            .find(|p| !p.is_empty() && !GENERIC.contains(&p.as_str()))
            .unwrap_or_default();
        let dashed = part.split_whitespace().collect::<Vec<_>>().join("-");
        let mut short = dashed.clone();
        while let Some((head, tail)) = short.rsplit_once('-')
            && GENERIC.contains(&tail)
        {
            short = head.to_string();
        }
        for n in [dashed, short] {
            if !n.is_empty() && !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
}

/// The first picture that exists (looked up once each), or none: Discord
/// then shows the Discord application's own icon.
fn find_icon(urls: &[String], known: &mut HashMap<String, bool>) -> Option<String> {
    urls.iter()
        .find(|url| {
            *known.entry((*url).clone()).or_insert_with(|| {
                std::process::Command::new("curl")
                    .args(["-sfI", "--max-time", "5", "-o", "/dev/null", url])
                    .status()
                    .is_ok_and(|s| s.success())
            })
        })
        .cloned()
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
    let mut known_icons = HashMap::new();
    let mut games = Games::default();
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
        // A game Discord knows: under its own application.
        let game_id = want
            .as_ref()
            .and_then(|a| a.game.as_ref())
            .and_then(|g| games.find(g));
        let client_id = match (&want, &conn) {
            (Some(a), _) => game_id.clone().unwrap_or_else(|| a.client_id.clone()),
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
        let icon = want
            .as_ref()
            .filter(|_| game_id.is_none())
            .and_then(|a| find_icon(&a.icons, &mut known_icons));
        match set_activity(stream, want.as_ref(), icon.as_deref(), game_id.is_some()) {
            Ok(()) => shown = Some(want.clone()),
            Err(e) => {
                eprintln!("seglinux-shell: Discord: {e}");
                conn = None;
            }
        }
    }
}

/// Discord's list of the games it knows (as its own game detection uses),
/// fetched once a week: Steam games by their Steam number, others by
/// title or program name. A game found there is shown as itself, with
/// its name and picture, as on Windows.
#[derive(Default)]
struct Games {
    loaded: bool,
    by_steam: HashMap<String, String>,
    by_name: HashMap<String, String>,
}

const GAME_LISTS: [&str; 2] = [
    "https://discord.com/api/v9/applications/detectable",
    // The same list, kept by arRPC (the open Discord RPC server).
    "https://raw.githubusercontent.com/OpenAsar/arrpc/main/src/process/detectable.json",
];

#[derive(serde::Deserialize)]
struct Detectable {
    id: String,
    name: String,
    #[serde(default)]
    aliases: Option<Vec<String>>,
    #[serde(default)]
    executables: Option<Vec<Executable>>,
    #[serde(default)]
    third_party_skus: Option<Vec<Sku>>,
}

#[derive(serde::Deserialize)]
struct Executable {
    name: String,
    #[serde(default)]
    is_launcher: bool,
}

#[derive(serde::Deserialize)]
struct Sku {
    distributor: String,
    id: Option<String>,
}

impl Games {
    fn find(&mut self, game: &Game) -> Option<String> {
        if !self.loaded {
            self.loaded = true;
            if let Some(list) = game_list() {
                self.index(list);
            }
        }
        game.steam
            .as_ref()
            .and_then(|s| self.by_steam.get(s))
            .or_else(|| game.names.iter().find_map(|n| self.by_name.get(n)))
            .cloned()
    }

    fn index(&mut self, list: Vec<Detectable>) {
        // A program name more than one game uses (launcher.exe) names none.
        let mut programs: HashMap<String, Option<String>> = HashMap::new();
        for game in list {
            for sku in game.third_party_skus.iter().flatten() {
                if sku.distributor == "steam"
                    && let Some(id) = &sku.id
                {
                    self.by_steam.entry(id.clone()).or_insert(game.id.clone());
                }
            }
            for name in std::iter::once(&game.name).chain(game.aliases.iter().flatten()) {
                self.by_name
                    .entry(name.to_lowercase())
                    .or_insert(game.id.clone());
            }
            for exe in game.executables.iter().flatten().filter(|e| !e.is_launcher) {
                let program = exe
                    .name
                    .rsplit('/')
                    .next()
                    .unwrap_or(&exe.name)
                    .to_lowercase();
                programs
                    .entry(program)
                    .and_modify(|id| {
                        if id.as_ref() != Some(&game.id) {
                            *id = None;
                        }
                    })
                    .or_insert(Some(game.id.clone()));
            }
        }
        for (program, id) in programs {
            if let Some(id) = id {
                self.by_name.entry(program).or_insert(id);
            }
        }
    }
}

/// The list, from the cache (~/.cache/seglinux) or downloaded again
/// when a week old.
fn game_list() -> Option<Vec<Detectable>> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
        .join("seglinux");
    let file = cache.join("discord-games.json");
    let week = Duration::from_secs(7 * 24 * 3600);
    let fresh = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < week));
    let read = |path: &std::path::Path| -> Option<Vec<Detectable>> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    };
    if !fresh {
        let _ = std::fs::create_dir_all(&cache);
        let part = file.with_extension("part");
        for url in GAME_LISTS {
            let got = std::process::Command::new("curl")
                .args(["-sfL", "--max-time", "120", "-o"])
                .arg(&part)
                .arg(url)
                .status()
                .is_ok_and(|s| s.success());
            if got && read(&part).is_some() {
                let _ = std::fs::rename(&part, &file);
                break;
            }
        }
        let _ = std::fs::remove_file(&part);
    }
    read(&file)
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

fn set_activity(
    s: &mut UnixStream,
    activity: Option<&Activity>,
    icon: Option<&str>,
    known_game: bool,
) -> io::Result<()> {
    let activity = activity.map(|a| {
        if known_game {
            // Discord shows the game's own name and picture.
            return serde_json::json!({ "type": 0, "timestamps": { "start": a.since } });
        }
        let mut v = serde_json::json!({
            "type": a.kind,
            "details": clip(&a.app),
            // The short status ("Playing Brave") names the application.
            "status_display_type": 2,
            "timestamps": { "start": a.since },
        });
        if let Some(icon) = icon {
            v["assets"] = serde_json::json!({ "large_image": icon, "large_text": clip(&a.app) });
        }
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
        assert_eq!(
            icon_names(["brave-browser", "org.mozilla.firefox", "Visual Studio Code"]),
            ["brave-browser", "brave", "firefox", "visual-studio-code"]
        );
        assert_eq!(
            icon_names(["com.spotify.Client", "/usr/share/pixmaps/steam.png"]),
            ["spotify", "steam"]
        );
        assert_eq!(clip(&"a".repeat(300)).len(), 128);
    }
}
