//! `seg clean`: orphans, the pacman cache, old logs and the size of
//! ~/.cache. A dry run unless `--apply` is given.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{Context, Error, Result};
use crate::sys::{self, bold, human_bytes};
use crate::vercmp::{self, parse_cache_file};

pub const HELP: &str = "\
Usage: seg clean [--apply] [--keep N] [--log-days N]

Finds what can go: packages nothing needs any more (pacman -Qdtq), cached
packages beyond the newest N versions (default 2, like paccache -k2),
rotated logs older than N days (default 30), and shows how big ~/.cache is.

By default nothing is changed: it only lists. --apply deletes (as root);
--dry-run is accepted for clarity and is the default.
";

#[derive(Debug)]
pub struct Options {
    pub apply: bool,
    pub keep: usize,
    pub log_days: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            apply: false,
            keep: 2,
            log_days: 30,
        }
    }
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Options> {
    use lexopt::prelude::*;
    let mut o = Options::default();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("apply") => o.apply = true,
            Long("dry-run") => o.apply = false,
            Long("keep") => o.keep = parser.value()?.parse()?,
            Long("log-days") => o.log_days = parser.value()?.parse()?,
            Short('h') | Long("help") => {
                out!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(other.unexpected().into()),
        }
    }
    Ok(o)
}

/// `CacheDir` lines of pacman.conf; the default when there are none.
pub fn parse_cache_dirs(conf: &str) -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = conf
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (key, value) = l.split_once('=')?;
            (key.trim() == "CacheDir").then(|| PathBuf::from(value.trim()))
        })
        .collect();
    if dirs.is_empty() {
        vec![PathBuf::from("/var/cache/pacman/pkg")]
    } else {
        dirs
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CachePlan {
    pub remove: Vec<PathBuf>,
    pub bytes: u64,
    pub kept: usize,
}

/// Which package files go: for every package (and architecture), all but the
/// newest `keep` versions, together with their signatures. `files` are
/// (path, size) pairs.
pub fn plan_cache(files: &[(PathBuf, u64)], keep: usize) -> CachePlan {
    // (name, arch) -> version -> files; signatures follow the package.
    type Versions<'a> = BTreeMap<String, Vec<(&'a Path, u64)>>;
    let mut groups: BTreeMap<(String, String), Versions> = BTreeMap::new();
    for (path, size) in files {
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(parsed) = parse_cache_file(file_name) else {
            continue;
        };
        groups
            .entry((parsed.name, parsed.arch))
            .or_default()
            .entry(parsed.version)
            .or_default()
            .push((path.as_path(), *size));
    }
    let mut plan = CachePlan::default();
    for versions in groups.into_values() {
        let mut versions: Vec<_> = versions.into_iter().collect();
        // Newest first.
        versions.sort_by(|a, b| vercmp::vercmp(&b.0, &a.0));
        for (index, (_, paths)) in versions.into_iter().enumerate() {
            if index < keep {
                plan.kept += 1;
                continue;
            }
            for (path, size) in paths {
                plan.remove.push(path.to_path_buf());
                plan.bytes += size;
            }
        }
    }
    plan.remove.sort();
    plan
}

fn cache_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (e.path(), meta.len()))
        })
        .collect()
}

/// Rotated logs: `messages.1`, `Xorg.0.log.old`, `syslog-20240101`,
/// `pacman.log.1.gz`, `auth.log-2024.xz`. The live log files are never
/// matched.
pub fn is_rotated_log(name: &str) -> bool {
    let compressed = [".gz", ".xz", ".zst", ".bz2", ".lz4"]
        .iter()
        .find_map(|ext| name.strip_suffix(ext));
    let stem = compressed.unwrap_or(name);
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if stem.ends_with(".old") {
        return true;
    }
    // "name.1", "name.12"
    if let Some((base, n)) = stem.rsplit_once('.')
        && !base.is_empty()
        && digits(n)
    {
        return true;
    }
    // "name-20240101", "name-2024"
    if let Some((base, date)) = stem.rsplit_once('-')
        && !base.is_empty()
        && date.len() >= 4
        && digits(date)
    {
        return true;
    }
    // A compressed copy of a log.
    compressed.is_some() && stem.contains(".log")
}

fn collect_old_logs(dir: &Path, depth: u32, older_than: Duration, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if depth > 0 {
                collect_old_logs(&path, depth - 1, older_than, out);
            }
            continue;
        }
        let rotated = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_rotated_log);
        let old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= older_than);
        if meta.is_file() && rotated && old {
            out.push((path, meta.len()));
        }
    }
}

fn orphans() -> Option<Vec<String>> {
    sys::which("pacman")?;
    // pacman -Qdtq exits 1 with no output when there are none.
    let out = sys::run("pacman", &["-Qdtq"]).ok()?;
    Some(
        out.stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

fn print_list(items: &[PathBuf]) {
    const SHOWN: usize = 8;
    for p in items.iter().take(SHOWN) {
        outln!("    {}", p.display());
    }
    if items.len() > SHOWN {
        outln!("    ... and {} more", items.len() - SHOWN);
    }
}

pub fn run(o: Options) -> Result<()> {
    if o.apply {
        sys::require_root("`seg clean --apply`")?;
        if Path::new("/var/lib/pacman/db.lck").exists() {
            return Err(Error::other(
                "pacman is locked (/var/lib/pacman/db.lck). If no pacman runs, `seg doctor` explains how to clear it.",
            ));
        }
    }
    let verb = if o.apply { "Removing" } else { "Would remove" };
    outln!(
        "{}",
        bold(if o.apply {
            "seg clean"
        } else {
            "seg clean (dry run: nothing is changed; --apply deletes)"
        })
    );

    // Orphans.
    let orphans = orphans();
    match &orphans {
        None => outln!("  orphans: pacman not found"),
        Some(list) if list.is_empty() => outln!("  orphans: none"),
        Some(list) => {
            outln!(
                "  orphans: {} packages nothing needs ({})",
                list.len(),
                list.join(" ")
            );
        }
    }

    // Package cache.
    let conf = fs::read_to_string("/etc/pacman.conf").unwrap_or_default();
    let mut cache = CachePlan::default();
    for dir in parse_cache_dirs(&conf) {
        let plan = plan_cache(&cache_files(&dir), o.keep);
        cache.kept += plan.kept;
        cache.bytes += plan.bytes;
        cache.remove.extend(plan.remove);
    }
    outln!(
        "  package cache: {verb} {} files ({}), keeping the newest {} versions of each package",
        cache.remove.len(),
        human_bytes(cache.bytes),
        o.keep
    );
    print_list(&cache.remove);

    // Logs.
    let mut logs = Vec::new();
    collect_old_logs(
        Path::new("/var/log"),
        3,
        Duration::from_secs(o.log_days * 86_400),
        &mut logs,
    );
    let log_bytes: u64 = logs.iter().map(|(_, s)| s).sum();
    let log_paths: Vec<PathBuf> = logs.into_iter().map(|(p, _)| p).collect();
    outln!(
        "  logs: {verb} {} rotated logs older than {} days ({})",
        log_paths.len(),
        o.log_days,
        human_bytes(log_bytes)
    );
    print_list(&log_paths);

    // ~/.cache: reported, never deleted.
    if let Some(home) = sys::invoking_home() {
        let cache_dir = home.join(".cache");
        let mut subdirs: Vec<(u64, String)> = fs::read_dir(&cache_dir)
            .map(|d| {
                d.flatten()
                    .map(|e| {
                        (
                            sys::dir_size(&e.path()),
                            e.file_name().to_string_lossy().into_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let total: u64 = subdirs.iter().map(|(s, _)| s).sum();
        outln!(
            "  {}: {} (never deleted by seg)",
            cache_dir.display(),
            human_bytes(total)
        );
        subdirs.sort_by(|a, b| b.cmp(a));
        for (size, name) in subdirs.into_iter().take(5) {
            outln!("    {:>10}  {name}", human_bytes(size));
        }
    }

    if o.apply {
        for path in cache.remove.iter().chain(&log_paths) {
            fs::remove_file(path).context(format!("removing {}", path.display()))?;
        }
        if let Some(list) = orphans.filter(|l| !l.is_empty()) {
            let mut args = vec!["-Rns", "--noconfirm"];
            args.extend(list.iter().map(String::as_str));
            sys::run_inherit("pacman", &args)?;
        }
        outln!("Done.");
    } else {
        let total = cache.bytes + log_bytes;
        outln!(
            "\nSegLinux could free {} of cache and logs. Run `sudo seg clean --apply` to do it.",
            human_bytes(total)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    fn files(names: &[&str]) -> Vec<(PathBuf, u64)> {
        names
            .iter()
            .map(|n| (PathBuf::from("/c").join(n), 100))
            .collect()
    }

    fn removed(plan: &CachePlan) -> Vec<String> {
        plan.remove
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn keeps_the_newest_two_versions() {
        let f = files(&[
            "foo-1.0-1-x86_64.pkg.tar.zst",
            "foo-1.0-1-x86_64.pkg.tar.zst.sig",
            "foo-1.1-1-x86_64.pkg.tar.zst",
            "foo-1.2-1-x86_64.pkg.tar.zst",
            "foo-1.10-1-x86_64.pkg.tar.zst",
            "bar-2.0-1-x86_64.pkg.tar.zst",
        ]);
        let plan = plan_cache(&f, 2);
        // 1.10 and 1.2 stay (1.10 is newer than 1.2, not older as text).
        assert_eq!(
            removed(&plan),
            [
                "foo-1.0-1-x86_64.pkg.tar.zst",
                "foo-1.0-1-x86_64.pkg.tar.zst.sig",
                "foo-1.1-1-x86_64.pkg.tar.zst"
            ]
        );
        assert_eq!(plan.bytes, 300);
        assert_eq!(plan.kept, 3);
    }

    #[test]
    fn epochs_and_releases_order_versions() {
        let f = files(&[
            "qt-6.7-1-x86_64.pkg.tar.zst",
            "qt-1:5.15-1-x86_64.pkg.tar.zst",
            "qt-6.7-2-x86_64.pkg.tar.zst",
        ]);
        let plan = plan_cache(&f, 1);
        // The epoch makes 5.15 the newest.
        assert_eq!(
            removed(&plan),
            ["qt-6.7-1-x86_64.pkg.tar.zst", "qt-6.7-2-x86_64.pkg.tar.zst"]
        );
    }

    #[test]
    fn architectures_and_names_are_separate_groups() {
        let f = files(&[
            "lib-1.0-1-x86_64.pkg.tar.zst",
            "lib-1.0-1-any.pkg.tar.zst",
            "lib-2.0-1-x86_64.pkg.tar.zst",
            "lib-git-1.0-1-x86_64.pkg.tar.zst",
        ]);
        assert!(
            plan_cache(&f, 1)
                .remove
                .iter()
                .all(|p| p.ends_with("lib-1.0-1-x86_64.pkg.tar.zst"))
        );
        assert!(plan_cache(&f, 2).remove.is_empty());
    }

    #[test]
    fn foreign_files_are_never_planned() {
        let f = files(&[
            "download-x",
            "foo-1.0-1-x86_64.pkg.tar.zst.part",
            "notes.txt",
        ]);
        assert_eq!(plan_cache(&f, 0), CachePlan::default());
    }

    #[test]
    fn keep_zero_removes_everything() {
        let f = files(&["a-1-1-any.pkg.tar.xz", "a-2-1-any.pkg.tar.xz"]);
        assert_eq!(plan_cache(&f, 0).remove.len(), 2);
    }

    #[test]
    fn cache_dirs_from_pacman_conf() {
        let conf = "[options]\n#CacheDir = /nope\nCacheDir = /var/cache/pacman/pkg/\nCacheDir=/mnt/cache\nHoldPkg = pacman\n";
        assert_eq!(
            parse_cache_dirs(conf),
            [
                PathBuf::from("/var/cache/pacman/pkg/"),
                PathBuf::from("/mnt/cache")
            ]
        );
        assert_eq!(
            parse_cache_dirs("[options]\n"),
            [PathBuf::from("/var/cache/pacman/pkg")]
        );
    }

    #[test]
    fn rotated_logs_only() {
        for yes in [
            "messages.1",
            "messages.12",
            "Xorg.0.log.old",
            "syslog-20240101.gz",
            "pacman.log.1.gz",
            "auth.log-2024.xz",
            "daemon.log.3.zst",
        ] {
            assert!(is_rotated_log(yes), "{yes}");
        }
        for no in [
            "messages",
            "pacman.log",
            "Xorg.0.log",
            "wtmp",
            "lastlog",
            "syslog.gz.bak",
            "1",
            ".1",
            "kern.log",
        ] {
            assert!(!is_rotated_log(no), "{no}");
        }
    }

    #[test]
    fn old_rotated_logs_are_found_below_the_depth_limit() {
        let t = TempDir::new();
        t.put("messages", "live");
        t.put("messages.1", "old");
        t.put("sub/a.log.2", "old");
        t.put("sub/deep/er/est/z.1", "too deep");
        let mut found = Vec::new();
        // Everything is brand new, so a zero age finds it and a day does not.
        collect_old_logs(t.path(), 3, Duration::from_secs(86_400), &mut found);
        assert!(found.is_empty());
        collect_old_logs(t.path(), 2, Duration::ZERO, &mut found);
        let mut names: Vec<_> = found
            .iter()
            .map(|(p, _)| {
                p.strip_prefix(t.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        assert_eq!(names, ["messages.1", "sub/a.log.2"]);
    }
}
