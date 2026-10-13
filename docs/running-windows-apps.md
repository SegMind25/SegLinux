# Running Windows programs

SegLinux runs Windows programs with [Wine](https://www.winehq.org/), plus
[DXVK](https://github.com/doitsujin/dxvk) and
[VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton) for
DirectX, and optionally Valve's Proton. `segrun` ties them together so
that a Windows program starts the way it would on Windows: double-click it.

## Starting a program

* **Double-click** an `.exe`, `.msi`, `.lnk` or `.bat` file in Dolphin or on
  the desktop.
* **Terminal**: `segrun setup.exe`, or `./setup.exe` after
  `chmod +x setup.exe` (the kernel hands files starting with `MZ` to segrun
  through binfmt_misc, see `/etc/binfmt.d/wine.conf`).
* **Installers without questions**: `segrun install --silent setup.exe`
  (Inno Setup, NSIS, InstallShield, WiX bundles, MSI and 7-Zip
  self-extractors are recognised).

Programs installed by an installer appear in the Start menu: Wine creates
the entries, just like Windows' Start menu folders.

## What segrun does

```
$ segrun info Game.exe
File:      /home/ada/Games/Game/Game.exe
Type:      64-bit x64 GUI program (PE32+)
Machine:   x64
Subsystem: GUI
Imports:   d3d11.dll, dxgi.dll, kernel32.dll, user32.dll, vcruntime140.dll, xinput1_3.dll

[info] Uses Direct3D 9/10/11: DXVK (Direct3D over Vulkan) gives the best speed.
[info] Uses Microsoft runtimes; if it complains about missing DLLs run: segrun prefix tricks <prefix> vcrun2022

SegLinux will run it with wine in prefix `default`:
  (once) Creating the Windows environment `default`
  (once) Installing DXVK into `default`
  WINEPREFIX=... WINEDEBUG=-all WINEESYNC=1 WINEFSYNC=1 wine /home/ada/Games/Game/Game.exe
```

1. **Identify the file** (`seglinux-pe`): PE32/PE32+, x86/x64/ARM64,
   GUI/console, DLL or driver, .NET, imported DLLs, installer framework,
   MSI, shortcut or batch file. DLLs, kernel drivers, EFI binaries and
   16-bit programs are refused with an explanation.
2. **Choose a Windows environment** (a Wine prefix, a separate "C: drive"):
   `--prefix NAME`, a matching `[[app]]` rule in the configuration, a
   per-program prefix with `--isolate` (or `isolate_installers = true`),
   otherwise the shared `default` prefix. Prefixes live in
   `~/.local/share/seglinux/prefixes/`.
3. **Prepare the prefix once**: `wineboot --init`, then DXVK when the
   program imports `d3d9`/`d3d10`/`d3d11`/`dxgi` and VKD3D-Proton when it
   imports `d3d12` (through winetricks; needs a Vulkan driver and network).
   What was installed is recorded in `.seglinux-prefix.toml` in the prefix.
4. **Start the program from its own folder**, as Windows does, with
   `WINEDEBUG=-all`, esync/fsync enabled, and your environment and DLL
   overrides. Games (programs that import Direct3D, OpenGL, Vulkan or
   XInput) run under GameMode; `--hud` adds the MangoHud overlay and
   `--gamescope` runs the program inside gamescope. See [gaming.md](gaming.md).

`segrun run --dry-run ...` prints all of this without running anything.

## Runners

| Runner | Setting | Notes |
|--------|---------|-------|
| Wine from the repositories | `wine` (default) | Wine 11 with NTSYNC |
| Another Wine build | `wine:/opt/wine-staging/bin/wine` | |
| Proton | `proton:/home/ada/.local/share/Steam/steamapps/common/Proton 9.0 (Beta)` | Ships its own DXVK/VKD3D-Proton |
| umu-launcher | `umu`, `umu:/path/to/GE-Proton` | Proton outside Steam (install umu-launcher first) |

Steam is installed: enable Proton for all titles in Steam → Settings →
Compatibility.

## Configuration

**Settings → Gaming** (Super+I) edits all of this with controls: the runner
(installed Proton builds are listed), the Windows environment, DXVK,
VKD3D-Proton, GameMode, gamescope, environment variables and per-game
profiles, keeping anything it has no control for. By hand:
`/etc/seglinux/segrun.toml` holds the defaults; copy it to
`~/.config/seglinux/segrun.toml` to change them. `segrun config` prints the
settings in effect.

```toml
[defaults]
runner = "wine"
prefix = "default"
isolate_installers = false
dxvk = "auto"      # auto | always | never
vkd3d = "auto"
esync = true
fsync = true

[env]
DXVK_ASYNC = "1"

[[app]]
match = "Photoshop*.exe"       # case-insensitive glob on the file name
prefix = "adobe"
dxvk = "never"
virtual_desktop = "1920x1080"
dll_overrides = ["d3dcompiler_47=n,b"]
args = ["-nosplash"]
env = { MESA_GL_VERSION_OVERRIDE = "4.6" }
```

## Commands

| Command | Windows counterpart |
|---------|---------------------|
| `segrun run FILE [ARGS]` (`install`, `open`) | Double-click |
| `segrun run --hud / --gamescope / --gamemode never FILE` | Game Bar overlay, fullscreen optimisations, Game Mode |
| `segrun info FILE` | File properties / compatibility tab |
| `segrun shortcut FILE [--name N]` | "Pin to Start" |
| `segrun prefix list / create / remove --yes / path / open` | Separate Windows installations, opening `C:\` |
| `segrun prefix tricks NAME VERBS...` | Installing the Visual C++ / .NET / DirectX runtimes |
| `segrun tool winecfg / regedit / taskmgr / control / explorer / cmd / uninstaller / notepad` | The Windows tools of the same name |
| `segrun doctor` | Compatibility troubleshooter |
| `sudo segrun binfmt register / unregister`, `segrun binfmt status` | |

## System tuning shipped by SegLinux

| File | Why |
|------|-----|
| `/etc/modules-load.d/ntsync.conf`, `/usr/lib/udev/rules.d/70-seglinux-ntsync.rules` | Loads NTSYNC (Linux 6.14+) and lets users open `/dev/ntsync`, which Wine 11 and Proton use for Windows synchronisation objects |
| `/etc/security/limits.d/90-seglinux-esync.conf` | File descriptor limit esync needs |
| `/etc/sysctl.d/80-seglinux-gaming.conf` | `vm.max_map_count` as on SteamOS |
| `/etc/binfmt.d/wine.conf` | `./program.exe` starts through segrun |
| `/etc/xdg/mimeapps.list` | Double-click opens Windows files with segrun |

## What does not work

* Programs that need **kernel drivers**: most anti-cheat systems (unless the
  game enables Proton support), copy protection drivers, VPN clients,
  hardware tools. `segrun info` warns when a program imports kernel modules.
* **ARM64** Windows programs on x86-64 PCs.
* **16-bit** programs: use DOSBox (`dosbox program.exe`), which is installed.

Check [WineHQ's application database](https://appdb.winehq.org/) and
[ProtonDB](https://www.protondb.com/) for program-specific advice.
