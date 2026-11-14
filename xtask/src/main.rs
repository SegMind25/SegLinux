//! `cargo xtask`: builds segboot and the kernel, writes a bootable GPT disk
//! image, runs it in QEMU.
//!
//! Only the Rust standard library is used: the GPT and FAT32 writers are in
//! `gpt.rs` and `fat.rs`. Running needs qemu-system-x86_64 and OVMF.

mod fat;
mod gpt;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  build     build segboot and the kernel
  image     build and write a bootable disk image (target/segos.img)
  run       build the image and boot it in QEMU (serial on this terminal)
  test      boot the kernel test suite in QEMU, fail if a test fails

options:
  --release        optimised build
  --headless       no QEMU window (run)
  --kernel <ELF>   put this kernel in the image instead of building segkernel

environment:
  OVMF      path to OVMF.fd (default /usr/share/ovmf/OVMF.fd)

The image is a GPT disk with one FAT32 EFI system partition; write it to a
USB stick with e.g. `dd if=target/segos.img of=/dev/sdX bs=4M`.
";

/// isa-debug-exit maps the kernel's success code 0x10 to (0x10 << 1) | 1.
const QEMU_TEST_SUCCESS: i32 = 33;
const TEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Disk layout: 1 MiB before the ESP (GPT, alignment), 64 MiB ESP, 1 MiB
/// after it (backup GPT).
const ESP_FIRST_LBA: u64 = 2048;
const ESP_SECTORS: u64 = 64 * 1024 * 1024 / gpt::SECTOR;
const DISK_SECTORS: u64 = ESP_FIRST_LBA + ESP_SECTORS + 2048;

struct Opts {
    release: bool,
    headless: bool,
    kernel: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let result = parse_opts(&args[1..]).and_then(|opts| match cmd.as_str() {
        "build" => build_all(&opts, false).map(|_| ()),
        "image" => image(&opts, false).map(|img| println!("{}", img.display())),
        "run" => image(&opts, false).and_then(|img| run(&img, &opts)),
        "test" => image(&opts, true).and_then(|img| test(&img)),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        release: false,
        headless: false,
        kernel: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--release" => opts.release = true,
            "--headless" => opts.headless = true,
            "--kernel" => {
                let p = it.next().ok_or("--kernel needs a path")?;
                opts.kernel = Some(PathBuf::from(p));
            }
            other => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
        }
    }
    Ok(opts)
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

fn cargo() -> Command {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root());
    cmd
}

fn profile(opts: &Opts) -> &'static str {
    if opts.release { "release" } else { "debug" }
}

fn build_bootloader(opts: &Opts) -> Result<PathBuf, String> {
    let mut cmd = cargo();
    cmd.args([
        "build",
        "--package",
        "segboot",
        "--target",
        "x86_64-unknown-uefi",
    ]);
    if opts.release {
        cmd.arg("--release");
    }
    status(&mut cmd)?;
    Ok(root()
        .join("target/x86_64-unknown-uefi")
        .join(profile(opts))
        .join("segboot.efi"))
}

fn build_kernel(opts: &Opts, ktest: bool) -> Result<PathBuf, String> {
    if let Some(k) = &opts.kernel {
        return Ok(k.clone());
    }
    let mut cmd = cargo();
    cmd.args([
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
    let dir = if ktest { "target/ktest" } else { "target" };
    Ok(root()
        .join(dir)
        .join("x86_64-unknown-none")
        .join(profile(opts))
        .join("segkernel"))
}

/// Builds segboot and the kernel; returns (bootloader, kernel).
fn build_all(opts: &Opts, ktest: bool) -> Result<(PathBuf, PathBuf), String> {
    Ok((build_bootloader(opts)?, build_kernel(opts, ktest)?))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    // RandomState is seeded from the OS's random source.
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(0);
        chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
    }
    out
}

fn image(opts: &Opts, ktest: bool) -> Result<PathBuf, String> {
    let (bootloader, kernel) = build_all(opts, ktest)?;
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()));
    let files = [
        ("EFI/BOOT/BOOTX64.EFI", read(&bootloader)?),
        ("boot/segkernel", read(&kernel)?),
    ];
    let name = if ktest { "segos-test.img" } else { "segos.img" };
    let path = root().join("target").join(name);
    let disk = disk_image(&files)?;
    std::fs::write(&path, disk).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// A GPT disk with one EFI system partition holding `files`.
fn disk_image(files: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let sector = gpt::SECTOR as usize;
    let mut disk = vec![0u8; DISK_SECTORS as usize * sector];
    let esp = ESP_FIRST_LBA as usize * sector..(ESP_FIRST_LBA + ESP_SECTORS) as usize * sector;
    let volume_id = u32::from_le_bytes(random_bytes());
    fat::format(&mut disk[esp], ESP_FIRST_LBA as u32, volume_id, files)?;
    gpt::write(
        &mut disk,
        gpt::random_guid(random_bytes()),
        &[gpt::Partition {
            type_guid: gpt::ESP_TYPE,
            guid: gpt::random_guid(random_bytes()),
            first_lba: ESP_FIRST_LBA,
            last_lba: ESP_FIRST_LBA + ESP_SECTORS - 1,
            name: "EFI System Partition",
        }],
    )?;
    Ok(disk)
}

fn qemu(img: &Path) -> Command {
    let mut cmd = Command::new("qemu-system-x86_64");
    let ovmf = std::env::var("OVMF").unwrap_or_else(|_| "/usr/share/ovmf/OVMF.fd".into());
    cmd.args(["-M", "q35", "-m", "512M", "-smp", "4", "-no-reboot"])
        .args(["-serial", "stdio"])
        .args(["-bios", &ovmf])
        .arg("-drive")
        .arg(format!("format=raw,file={}", img.display()));
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

fn run(img: &Path, opts: &Opts) -> Result<(), String> {
    let mut cmd = qemu(img);
    if opts.headless {
        cmd.args(["-display", "none"]);
    }
    status(&mut cmd)
}

fn test(img: &Path) -> Result<(), String> {
    let mut child = qemu(img)
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
