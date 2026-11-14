# SegOS

An operating system written from scratch in Rust, aiming to run **unmodified
Linux applications** (and, through Wine/Proton, Windows ones) by implementing
the Linux system call interface in a new kernel.

Status: **Phase 1 (multitasking) done**. The kernel boots on UEFI and BIOS,
prints to the serial port and the screen, handles CPU exceptions, manages
physical memory, page tables and a kernel heap, starts every CPU, and runs
kernel threads on all of them with a preemptive scheduler. It does not run
programs yet; that is Phase 2.

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
| `kernel/src/hal/` | x86-64: serial, GDT/TSS, IDT and the trap path, local APIC and timer, per-CPU data, starting CPUs |
| `kernel/src/mm/` | frame allocator, page tables, kernel heap |
| `kernel/src/task/` | kernel threads, scheduler and context switch, wait queues |
| `kernel/src/sync.rs` | interrupt-safe spinlock, sleeping mutex |
| `kernel/src/time.rs` | uptime and sleeping |
| `kernel/src/console.rs` | framebuffer text console and kernel log |
| `kernel/src/limine.rs` | boot protocol requests |
| `kernel/src/ktest.rs` | in-kernel tests |
| `xtask/` | build tool: kernel, ISO, QEMU |
| `image/` | bootloader configuration |
| `site/` | the website, deployed by Vercel (`vercel.json`) |

Subsystems become their own crates once they grow (see the plan).

## Licence

MIT or Apache-2.0, at your option.

The previous project in this repository, SegLinux (a Linux distribution),
remains on the `main` branch.
