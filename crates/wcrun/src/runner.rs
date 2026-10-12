//! Compatibility runners: plain Wine, Valve's Proton, or umu-launcher.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use wincraft_core::cmd::Cmd;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runner {
    /// A Wine build: `wine` from the repositories or a custom binary.
    Wine { bin: String },
    /// A Proton directory (the one containing the `proton` script).
    Proton { dir: PathBuf },
    /// `umu-run`, optionally pinned to a Proton build via `PROTONPATH`.
    Umu { proton: Option<PathBuf> },
}

impl Runner {
    pub fn kind(&self) -> &'static str {
        match self {
            Runner::Wine { .. } => "wine",
            Runner::Proton { .. } => "proton",
            Runner::Umu { .. } => "umu",
        }
    }

    /// Proton and umu ship their own DXVK and VKD3D-Proton.
    pub fn bundles_dxvk(&self) -> bool {
        !matches!(self, Runner::Wine { .. })
    }

    /// The `WINEPREFIX` for a Wincraft prefix directory. Proton keeps the
    /// actual prefix in a `pfx` subdirectory of its compat data path.
    pub fn wineprefix(&self, prefix_dir: &Path) -> PathBuf {
        match self {
            Runner::Proton { .. } => prefix_dir.join("pfx"),
            _ => prefix_dir.to_path_buf(),
        }
    }

    /// Command that runs `inner` (a Windows command line) in the prefix.
    pub fn command(&self, prefix_dir: &Path, inner: &[String]) -> Cmd {
        match self {
            Runner::Wine { bin } => Cmd::new(bin.clone())
                .args(inner.iter().cloned())
                .env("WINEPREFIX", path_str(prefix_dir)),
            Runner::Proton { dir } => Cmd::new(path_str(&dir.join("proton")))
                .arg("run")
                .args(inner.iter().cloned())
                .env("STEAM_COMPAT_DATA_PATH", path_str(prefix_dir))
                .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", path_str(&steam_root()))
                .env("WINEPREFIX", path_str(&prefix_dir.join("pfx"))),
            Runner::Umu { proton } => {
                let mut cmd = Cmd::new("umu-run")
                    .args(inner.iter().cloned())
                    .env("WINEPREFIX", path_str(prefix_dir))
                    .env("GAMEID", "umu-default");
                if let Some(p) = proton {
                    cmd = cmd.env("PROTONPATH", path_str(p));
                }
                cmd
            }
        }
    }

    /// The program that must be installed for this runner to work.
    pub fn required_program(&self) -> String {
        match self {
            Runner::Wine { bin } => bin.clone(),
            Runner::Proton { dir } => path_str(&dir.join("proton")),
            Runner::Umu { .. } => "umu-run".to_string(),
        }
    }

    /// The Wine binary winetricks should use (`WINE=`).
    pub fn wine_binary(&self) -> Option<&str> {
        match self {
            Runner::Wine { bin } => Some(bin),
            _ => None,
        }
    }
}

fn steam_root() -> PathBuf {
    let data = wincraft_core::xdg::data_home().join("Steam");
    if data.exists() {
        return data;
    }
    let legacy = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".steam/steam");
    if legacy.exists() { legacy } else { data }
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl FromStr for Runner {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, arg) = match s.split_once(':') {
            Some((k, a)) => (k, Some(a)),
            None => (s, None),
        };
        let arg = arg.filter(|a| !a.is_empty());
        match (kind.to_ascii_lowercase().as_str(), arg) {
            ("wine", None) => Ok(Runner::Wine { bin: "wine".into() }),
            ("wine", Some(bin)) => Ok(Runner::Wine { bin: bin.into() }),
            ("proton", Some(dir)) => Ok(Runner::Proton { dir: dir.into() }),
            ("proton", None) => Err("`proton` needs a path, e.g. proton:/path/to/Proton".into()),
            ("umu", proton) => Ok(Runner::Umu {
                proton: proton.map(PathBuf::from),
            }),
            _ => Err(format!(
                "unknown runner `{s}` (expected wine, wine:<binary>, proton:<dir>, umu or umu:<proton dir>)"
            )),
        }
    }
}

impl fmt::Display for Runner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Runner::Wine { bin } if bin == "wine" => f.write_str("wine"),
            Runner::Wine { bin } => write!(f, "wine:{bin}"),
            Runner::Proton { dir } => write!(f, "proton:{}", dir.display()),
            Runner::Umu { proton: None } => f.write_str("umu"),
            Runner::Umu { proton: Some(p) } => write!(f, "umu:{}", p.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        for s in [
            "wine",
            "wine:/opt/wine-staging/bin/wine",
            "proton:/games/Proton 9.0",
            "umu",
            "umu:/p/GE-Proton",
        ] {
            assert_eq!(s.parse::<Runner>().unwrap().to_string(), s);
        }
        assert!("proton".parse::<Runner>().is_err());
        assert!("bottles".parse::<Runner>().is_err());
    }

    #[test]
    fn wine_command() {
        let r: Runner = "wine".parse().unwrap();
        let cmd = r.command(Path::new("/p"), &["winecfg".to_string()]);
        assert_eq!(cmd.to_string(), "WINEPREFIX=/p wine winecfg");
        assert!(!r.bundles_dxvk());
    }

    #[test]
    fn proton_uses_compat_data_path() {
        let r: Runner = "proton:/opt/proton".parse().unwrap();
        let cmd = r.command(Path::new("/p"), &["app.exe".to_string()]);
        assert_eq!(cmd.program, "/opt/proton/proton");
        assert_eq!(cmd.args, vec!["run", "app.exe"]);
        assert_eq!(cmd.get_env("STEAM_COMPAT_DATA_PATH"), Some("/p"));
        assert_eq!(cmd.get_env("WINEPREFIX"), Some("/p/pfx"));
        assert_eq!(r.wineprefix(Path::new("/p")), Path::new("/p/pfx"));
        assert!(r.bundles_dxvk());
    }

    #[test]
    fn umu_sets_protonpath() {
        let r: Runner = "umu:/opt/ge".parse().unwrap();
        let cmd = r.command(Path::new("/p"), &["app.exe".to_string()]);
        assert_eq!(cmd.program, "umu-run");
        assert_eq!(cmd.get_env("PROTONPATH"), Some("/opt/ge"));
        assert_eq!(cmd.get_env("GAMEID"), Some("umu-default"));
    }
}
