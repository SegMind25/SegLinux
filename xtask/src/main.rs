//! `cargo xtask`: builds the kernel, makes a bootable ISO, runs it in QEMU.
//!
//! Needs: qemu-system-x86_64, xorriso, git, make, a C compiler (for the
//! Limine install tool) and OVMF for UEFI boot.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  build     build the kernel
  image     build the kernel and a bootable ISO (target/segos.iso)
  run       build the ISO and boot it in QEMU (serial on this terminal)
  test      boot the kernel test suite in QEMU, fail if a test fails

options:
  --release   optimised kernel
  --bios      boot with legacy BIOS instead of UEFI (run)
  --headless  no QEMU window (run)

environment:
  OVMF        path to OVMF.fd (default /usr/share/ovmf/OVMF.fd)
  LIMINE_DIR  existing Limine binary release (default: cloned into target/)
";

const LIMINE_BRANCH: &str = "v9.x-binary";
const LIMINE_REPO: &str = "https://github.com/limine-bootloader/limine.git";
/// isa-debug-exit maps the kernel's success code 0x10 to (0x10 << 1) | 1.
const QEMU_TEST_SUCCESS: i32 = 33;
const TEST_TIMEOUT: Duration = Duration::from_secs(300);

struct Opts {
    release: bool,
    bios: bool,
    headless: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let has = |f: &str| args.iter().any(|a| a == f);
    let opts = Opts {
        release: has("--release"),
        bios: has("--bios"),
        headless: has("--headless"),
    };
    let result = match cmd.as_str() {
        "build" => build(&opts, false).map(|_| ()),
        "image" => image(&opts, false).map(|iso| println!("{}", iso.display())),
        "run" => image(&opts, false).and_then(|iso| run(&iso, &opts)),
        "test" => image(&opts, true).and_then(|iso| test(&iso)),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn status(cmd: &mut Command) -> Result<(), String> {
    let s = cmd
        .status()
        .map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed: {s}"))
    }
}

fn build(opts: &Opts, ktest: bool) -> Result<PathBuf, String> {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root()).args([
        "build",
        "--package",
        "segkernel",
        "--target",
        "x86_64-unknown-none",
    ]);
    if opts.release {
        cmd.arg("--release");
    }
    if ktest {
        // Separate target dir: switching features must not rebuild the normal kernel.
        cmd.args(["--features", "ktest", "--target-dir", "target/ktest"]);
    }
    status(&mut cmd)?;
    let profile = if opts.release { "release" } else { "debug" };
    let dir = if ktest { "target/ktest" } else { "target" };
    Ok(root()
        .join(dir)
        .join("x86_64-unknown-none")
        .join(profile)
        .join("segkernel"))
}

fn limine_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var("LIMINE_DIR").ok().filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let dir = root().join("target/limine");
    if !dir.join("BOOTX64.EFI").exists() {
        status(
            Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--branch",
                    LIMINE_BRANCH,
                    LIMINE_REPO,
                ])
                .arg(&dir),
        )?;
    }
    if !dir.join("limine").exists() {
        status(Command::new("make").arg("-C").arg(&dir))?;
    }
    Ok(dir)
}

fn image(opts: &Opts, ktest: bool) -> Result<PathBuf, String> {
    let kernel = build(opts, ktest)?;
    let limine = limine_dir()?;
    let name = if ktest { "segos-test" } else { "segos" };
    let staging = root().join("target").join(format!("{name}-iso"));
    let _ = std::fs::remove_dir_all(&staging);
    let boot = staging.join("boot/limine");
    let efi = staging.join("EFI/BOOT");
    for d in [&boot, &efi] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let copy = |from: &Path, to: &Path| {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| format!("copy {}: {e}", from.display()))
    };
    copy(&kernel, &staging.join("boot/segkernel"))?;
    copy(&root().join("image/limine.conf"), &boot.join("limine.conf"))?;
    for f in [
        "limine-bios.sys",
        "limine-bios-cd.bin",
        "limine-uefi-cd.bin",
    ] {
        copy(&limine.join(f), &boot.join(f))?;
    }
    copy(&limine.join("BOOTX64.EFI"), &efi.join("BOOTX64.EFI"))?;

    let iso = root().join("target").join(format!("{name}.iso"));
    status(
        Command::new("xorriso")
            .args(["-as", "mkisofs", "-quiet", "-R", "-r", "-J"])
            .args([
                "-b",
                "boot/limine/limine-bios-cd.bin",
                "-no-emul-boot",
                "-boot-load-size",
                "4",
            ])
            .args(["-boot-info-table", "-hfsplus", "-apm-block-size", "2048"])
            .args([
                "--efi-boot",
                "boot/limine/limine-uefi-cd.bin",
                "-efi-boot-part",
            ])
            .args(["--efi-boot-image", "--protective-msdos-label"])
            .arg(&staging)
            .arg("-o")
            .arg(&iso),
    )?;
    status(
        Command::new(limine.join("limine"))
            .arg("bios-install")
            .arg(&iso),
    )?;
    Ok(iso)
}

fn qemu(iso: &Path, bios: bool) -> Command {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-M", "q35", "-m", "512M", "-smp", "4", "-no-reboot"])
        .args(["-serial", "stdio"])
        .arg("-cdrom")
        .arg(iso);
    if !bios {
        let ovmf = std::env::var("OVMF").unwrap_or_else(|_| "/usr/share/ovmf/OVMF.fd".into());
        cmd.args(["-bios", &ovmf]);
    }
    // Usable KVM only: CI runners often have /dev/kvm without permission.
    if std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
    {
        cmd.args(["-enable-kvm", "-cpu", "host"]);
    }
    cmd
}

fn run(iso: &Path, opts: &Opts) -> Result<(), String> {
    let mut cmd = qemu(iso, opts.bios);
    if opts.headless {
        cmd.args(["-display", "none"]);
    }
    status(&mut cmd)
}

fn test(iso: &Path) -> Result<(), String> {
    let mut child = qemu(iso, false)
        .args(["-display", "none"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .spawn()
        .map_err(|e| format!("cannot start QEMU: {e}"))?;
    let start = std::time::Instant::now();
    loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            return match s.code() {
                Some(QEMU_TEST_SUCCESS) => Ok(()),
                code => Err(format!("kernel tests failed (QEMU exit code {code:?})")),
            };
        }
        if start.elapsed() > TEST_TIMEOUT {
            let _ = child.kill();
            return Err(format!("kernel tests timed out after {TEST_TIMEOUT:?}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
