# SegOS

An operating system written from scratch in Rust, aiming to run **unmodified
Linux applications** (and, through Wine/Proton, Windows ones) by implementing
the Linux system call interface in a new kernel.

Status: **Phase 0 (bring-up)**. The kernel boots on UEFI and BIOS, prints to
the serial port and the screen, handles CPU exceptions, and manages physical
memory, page tables and a kernel heap. It does not run programs yet.

The whole roadmap, from here to Steam: [docs/plan.md](docs/plan.md).

## Build and run

Needs Rust (the version is pinned in `rust-toolchain.toml`), QEMU, xorriso,
git, make, a C compiler and OVMF. On Debian/Ubuntu:

```bash
sudo apt install qemu-system-x86 xorriso ovmf build-essential git
```

```bash
cargo xtask run          # build the ISO and boot it in QEMU (UEFI)
```

```bash
cargo xtask run --bios   # legacy BIOS boot
```

```bash
cargo xtask test         # in-kernel tests, in QEMU
```

```bash
cargo xtask image        # target/segos.iso, bootable from USB or DVD
```

The first build downloads the [Limine](https://github.com/limine-bootloader/limine)
bootloader into `target/limine`.

## Layout

| Path | What |
|---|---|
| `kernel/src/hal/` | x86-64: serial, GDT/TSS, IDT and the trap path, CPU helpers |
| `kernel/src/mm/` | frame allocator, page tables, kernel heap |
| `kernel/src/console.rs` | framebuffer text console and kernel log |
| `kernel/src/limine.rs` | boot protocol requests |
| `kernel/src/ktest.rs` | in-kernel tests |
| `xtask/` | build tool: kernel, ISO, QEMU |
| `image/` | bootloader configuration |

Subsystems become their own crates once they grow (see the plan).

## Licence

MIT or Apache-2.0, at your option.

The previous project in this repository, SegLinux (a Linux distribution),
remains on the `main` branch.
