//! Compatibility hints derived from what a program imports.

use std::fmt;

use wincraft_pe::{FileKind, Machine, PeInfo, Subsystem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    pub level: Level,
    pub text: String,
}

impl fmt::Display for Advice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self.level {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        };
        write!(f, "[{tag}] {}", self.text)
    }
}

const D3D_DXVK: &[&str] = &[
    "d3d9.dll",
    "d3d10.dll",
    "d3d10_1.dll",
    "d3d10core.dll",
    "d3d11.dll",
    "dxgi.dll",
];
const D3D_VKD3D: &[&str] = &["d3d12.dll", "d3d12core.dll"];

/// Whether the program renders with Direct3D 9-11 (DXVK territory).
pub fn uses_d3d_dxvk(info: &PeInfo) -> bool {
    info.imports_matching(|i| D3D_DXVK.contains(&i))
}

/// Whether the program renders with Direct3D 12 (VKD3D-Proton territory).
pub fn uses_d3d12(info: &PeInfo) -> bool {
    info.imports_matching(|i| D3D_VKD3D.contains(&i))
}

/// Whether the program looks like a game: it renders with Direct3D,
/// DirectDraw, OpenGL or Vulkan, or reads game controllers. Installers are
/// never games, even when their wizard uses Direct3D.
pub fn is_game(info: &PeInfo) -> bool {
    info.installer.is_none()
        && info.is_launchable()
        && info.imports_matching(|i| {
            D3D_DXVK.contains(&i)
                || D3D_VKD3D.contains(&i)
                || matches!(
                    i,
                    "d3d8.dll" | "ddraw.dll" | "opengl32.dll" | "vulkan-1.dll" | "dinput8.dll"
                )
                || i.starts_with("xinput")
                || i.starts_with("xaudio2")
        })
}

/// Visual C++ runtimes and the winetricks verb that installs each one.
const VCRUN: &[(&str, &str)] = &[
    ("msvcr80.dll", "vcrun2005"),
    ("msvcp80.dll", "vcrun2005"),
    ("msvcr90.dll", "vcrun2008"),
    ("msvcp90.dll", "vcrun2008"),
    ("msvcr100.dll", "vcrun2010"),
    ("msvcp100.dll", "vcrun2010"),
    ("msvcr110.dll", "vcrun2012"),
    ("msvcp110.dll", "vcrun2012"),
    ("msvcr120.dll", "vcrun2013"),
    ("msvcp120.dll", "vcrun2013"),
    ("vcruntime140.dll", "vcrun2022"),
    ("vcruntime140_1.dll", "vcrun2022"),
    ("msvcp140.dll", "vcrun2022"),
];

/// winetricks verbs that commonly help this program.
pub fn suggested_verbs(info: &PeInfo) -> Vec<&'static str> {
    let mut verbs: Vec<&'static str> = Vec::new();
    let mut add = |v: &'static str| {
        if !verbs.contains(&v) {
            verbs.push(v);
        }
    };
    for (dll, verb) in VCRUN {
        if info.imports_dll(dll) {
            add(verb);
        }
    }
    if info.imports_dll("d3dcompiler_47.dll") {
        add("d3dcompiler_47");
    }
    if info.imports_matching(|i| i.starts_with("d3dx9_")) {
        add("d3dx9");
    }
    if info.imports_matching(|i| i.starts_with("physxloader") || i.starts_with("physx3")) {
        add("physx");
    }
    verbs
}

pub fn advise(kind: &FileKind) -> Vec<Advice> {
    let mut out = Vec::new();
    let mut push = |level, text: String| out.push(Advice { level, text });

    match kind {
        FileKind::Pe(info) => {
            if info.is_dll {
                push(Level::Error, "This is a library (DLL), not a program. Run the program that uses it instead.".into());
            }
            match info.subsystem {
                Subsystem::Native => push(
                    Level::Error,
                    "This is a Windows kernel driver. Drivers (including most anti-cheat and copy-protection drivers) cannot run under Wine.".into(),
                ),
                Subsystem::EfiApplication | Subsystem::EfiDriver => {
                    push(Level::Error, "This is a UEFI firmware binary, not a Windows program.".into())
                }
                Subsystem::WindowsCui => push(
                    Level::Info,
                    "Console program: run it from a terminal to see its output.".into(),
                ),
                _ => {}
            }
            match info.machine {
                Machine::Arm64 | Machine::Arm => push(
                    Level::Warn,
                    format!("{} binary: it needs a Wine build for that architecture; x86 Wincraft cannot run it.", info.machine),
                ),
                Machine::Other(_) => push(Level::Error, "Unknown CPU architecture.".into()),
                _ => {}
            }
            if info.dotnet {
                push(
                    Level::Info,
                    ".NET program: Wine Mono runs most of these. If it fails, try `wcrun prefix tricks <prefix> dotnet48` in an isolated prefix.".into(),
                );
            }
            if uses_d3d_dxvk(info) {
                push(Level::Info, "Uses Direct3D 9/10/11: DXVK (Direct3D over Vulkan) gives the best speed.".into());
            }
            if uses_d3d12(info) {
                push(Level::Info, "Uses Direct3D 12: needs VKD3D-Proton and a Vulkan 1.3 GPU driver.".into());
            }
            if info.imports_dll("opengl32.dll") {
                push(Level::Info, "Uses OpenGL: runs through Wine directly on your Mesa/NVIDIA driver.".into());
            }
            if info.imports_matching(|i| i == "ntoskrnl.exe" || i == "fltmgr.sys") && !info.is_dll {
                push(Level::Warn, "Imports kernel-mode modules: anti-cheat or driver installers usually do not work.".into());
            }
            let verbs = suggested_verbs(info);
            if !verbs.is_empty() {
                push(
                    Level::Info,
                    format!(
                        "Uses Microsoft runtimes; if it complains about missing DLLs run: wcrun prefix tricks <prefix> {}",
                        verbs.join(" ")
                    ),
                );
            }
            if let Some(kind) = info.installer {
                push(
                    Level::Info,
                    format!(
                        "{kind} installer: `wcrun install --silent` passes {} for an unattended install.",
                        kind.silent_args().join(" ")
                    ),
                );
            }
        }
        FileKind::Dos => push(
            Level::Warn,
            "16-bit DOS/Windows 3.x program: modern 64-bit Wine cannot run these; use DOSBox (`dosbox <file>`).".into(),
        ),
        FileKind::Msi => push(Level::Info, "Windows Installer package: installed with msiexec inside the prefix.".into()),
        FileKind::Shortcut => push(Level::Info, "Shortcut: opened with Wine's `start` command.".into()),
        FileKind::Batch => push(Level::Info, "Batch script: runs with Wine's cmd.exe.".into()),
        FileKind::OleCompound => push(Level::Error, "This is an Office/OLE document, not a program.".into()),
        FileKind::Unknown => push(Level::Error, "Not a Windows program.".into()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wincraft_pe::PeFormat;

    fn info(imports: &[&str]) -> PeInfo {
        PeInfo {
            machine: Machine::Amd64,
            format: PeFormat::Pe32Plus,
            subsystem: Subsystem::WindowsGui,
            is_dll: false,
            large_address_aware: true,
            timestamp: 0,
            dotnet: false,
            imports: imports.iter().map(|s| s.to_string()).collect(),
            sections: vec![],
            overlay_size: 0,
            installer: None,
        }
    }

    #[test]
    fn d3d_detection() {
        assert!(uses_d3d_dxvk(&info(&["d3d11.dll", "kernel32.dll"])));
        assert!(uses_d3d_dxvk(&info(&["d3d9.dll"])));
        assert!(!uses_d3d_dxvk(&info(&["opengl32.dll"])));
        assert!(uses_d3d12(&info(&["d3d12.dll"])));
    }

    #[test]
    fn game_detection() {
        assert!(is_game(&info(&["d3d11.dll"])));
        assert!(is_game(&info(&["opengl32.dll"])));
        assert!(is_game(&info(&["xinput1_4.dll"])));
        assert!(!is_game(&info(&["user32.dll", "comctl32.dll"])));
        let mut installer = info(&["d3d9.dll"]);
        installer.installer = Some(wincraft_pe::InstallerKind::Nsis);
        assert!(!is_game(&installer));
    }

    #[test]
    fn verbs_are_deduplicated() {
        let i = info(&[
            "msvcp140.dll",
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "d3dx9_43.dll",
        ]);
        assert_eq!(suggested_verbs(&i), vec!["vcrun2022", "d3dx9"]);
    }

    #[test]
    fn dll_and_driver_errors() {
        let mut dll = info(&[]);
        dll.is_dll = true;
        let advice = advise(&FileKind::Pe(Box::new(dll)));
        assert!(advice.iter().any(|a| a.level == Level::Error));

        let mut drv = info(&["ntoskrnl.exe"]);
        drv.subsystem = Subsystem::Native;
        let advice = advise(&FileKind::Pe(Box::new(drv)));
        assert!(advice.iter().any(|a| a.text.contains("kernel driver")));
    }
}
