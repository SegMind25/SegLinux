# SegOS: a from-scratch operating system in Rust (plan)

Status: **Phase 0 done** (boots on UEFI and BIOS; exceptions, frames,
paging and heap work, 7 kernel tests pass in QEMU). Next: Phase 1.

## 1. The goal, stated precisely

"Run every application" decides the whole design. The applications people
use (Steam, Firefox, Chromium, Discord, Blender, LibreOffice, Wine/Proton for
Windows games) exist as **Linux x86-64 binaries**. Nobody will port them to a
new OS. So the new kernel must be **binary-compatible with Linux**: it loads
the same ELF files, answers the same system calls with the same numbers,
structures and error codes, and provides the same `/proc`, `/sys` and `/dev`
files those programs read.

That is the approach of gVisor, Asterinas, Kerla and Maestro, and of
FreeBSD's and Windows' (WSL1) Linux layers. It is the only realistic route
to "every application" for a new kernel.

Windows programs then come for free in the same way they do today: Wine and
Proton are Linux programs.

Non-goals (at least for the first years): running Linux kernel modules,
NVIDIA's proprietary driver, Secure Boot signed by Microsoft, 32-bit-only
hardware, architectures other than x86-64 (aarch64 later).

## 2. Architecture

```
 ┌──────────────────────────────────────────────────────────────┐
 │ Unmodified Linux apps: bash, coreutils, Mesa, Firefox, Steam │
 │ Wine/Proton, SegLinux's Rust compositor and shell            │
 ├──────────────────────────────────────────────────────────────┤
 │ glibc / musl (unmodified)                                    │
 ╞══════════════════════ syscall boundary (Linux ABI) ══════════╡
 │ segkernel (Rust, no_std)                                     │
 │  linux-abi: syscall table, errno, struct layouts, ioctls     │
 │  process:  tasks, threads, signals, futex, ptrace, cgroups*  │
 │  mm:       paging, mmap, COW fork, page cache, swap*         │
 │  vfs:      dentries, mounts, ext4, tmpfs, procfs, sysfs, dev │
 │  net:      sockets, unix, TCP/IP (smoltcp first), netlink    │
 │  drivers:  virtio, NVMe, AHCI, xHCI/USB HID, e1000, DRM/KMS  │
 │  sched:    SMP, per-CPU run queues, preemption               │
 ├──────────────────────────────────────────────────────────────┤
 │ segkernel-hal: x86-64 only (the one place for `unsafe` asm)  │
 │  GDT/IDT, APIC/x2APIC, HPET/TSC, ACPI, PCI(e), IOMMU*        │
 ╞══════════════════════════════════════════════════════════════╡
 │ Bootloader: UEFI (Limine protocol), also Limine on BIOS      │
 └──────────────────────────────────────────────────────────────┘
        * later phases
```

Design decisions:

- **Monolithic kernel with a small unsafe core** ("framekernel", as in
  Asterinas). All `unsafe` code lives in `segkernel-hal` and a few audited
  modules; everything else is `#![forbid(unsafe_code)]`. This keeps Linux-like
  performance (no IPC per syscall, which a microkernel would pay on every
  `read()` from a game) while getting most of Rust's safety.
- **Linux ABI is the native ABI**, not an emulation layer. There is no
  second "own" syscall interface to maintain.
- **Drivers expose Linux's userspace interfaces**: DRM/KMS ioctls, evdev
  (`/dev/input/event*`), ALSA. Then Mesa, libinput, PipeWire and SegLinux's
  existing Smithay compositor work without changes.
- **Boot via Limine** (UEFI + BIOS), so no bootloader is written in Phase 0.
- **Toolchain**: stable Rust pinned in `rust-toolchain.toml`, built-in target
  `x86_64-unknown-none`, built with `cargo xtask` (build, run in QEMU, test).

## 3. Repository layout (target)

Phase 0 keeps everything in one `kernel` crate with `hal/` and `mm/`
modules; they move into the crates below as they grow.

```
Cargo.toml              workspace
rust-toolchain.toml
xtask/                  cargo xtask build|run|test|image
kernel/
  hal/                  x86-64: boot entry, interrupts, paging, APIC, ACPI
  mm/                   frame allocator, page tables, VMAs, page cache
  sched/                tasks, SMP scheduler, timers
  vfs/                  VFS + tmpfs, devfs, procfs, sysfs, ext4, fat32
  net/                  socket layer, unix sockets, TCP/IP
  linux-abi/            syscall dispatch, every syscall in its own module
  drivers/              virtio-*, nvme, ahci, xhci, hid, e1000, gpu
  kernel/               the binary that ties it together
userland/
  init/                 segos-init, PID 1, Rust
  tests/                syscall conformance tests (run on Linux AND SegOS)
image/                  builds the disk image: Limine + kernel + rootfs
docs/
```

The rootfs is not written from scratch: it uses unmodified binaries
(BusyBox/musl first, then a glibc-based distribution root such as Artix or
Alpine) so every phase is measured against real software.

## 4. Roadmap

Each phase ends with a demo that must work in QEMU and in CI.

| Phase | What gets built | Done when | Rough effort* |
|---|---|---|---|
| 0. Bring-up | Workspace, xtask, Limine boot, serial + framebuffer console, GDT/IDT, exceptions, physical frame allocator, kernel heap, paging | Kernel prints "SegOS" in QEMU, panics show a backtrace, CI boots it | 2–4 weeks |
| 1. Multitasking | APIC timer, kernel threads, preemptive scheduler, SMP bring-up, spinlocks/mutexes | 4 CPUs run kernel threads in parallel | 1–2 months |
| 2. First user process | Ring 3, `syscall`/`sysret`, ELF loader, address spaces, initramfs (cpio), tmpfs, ~40 syscalls (`read`, `write`, `mmap`, `brk`, `exit`, …) | A static musl "hello world" built on Linux runs unmodified | 1–2 months |
| 3. A shell | `fork` with copy-on-write, `execve`, `wait4`, pipes, signals, TTY + line discipline, `ioctl(TCGETS…)`, devfs, procfs basics | BusyBox `sh` runs; `ls \| grep \| wc` works | 2–3 months |
| 4. Real storage | PCI, virtio-blk, NVMe, AHCI, page cache, ext4 (read, then write), fat32 for the EFI partition | Boots from a disk with a full rootfs | 2–3 months |
| 5. glibc and dynamic linking | `ld-linux.so`, threads (`clone` flags, TLS, `futex`, `set_robust_list`), `epoll`, `eventfd`, `timerfd`, `signalfd`, `poll/select`, `/proc/self/*` | Unmodified Debian/Artix userland: bash, coreutils, python3, gcc compiling a program | 3–4 months |
| 6. Networking | virtio-net, e1000, TCP/IP (smoltcp, later own), unix sockets incl. `SCM_RIGHTS`, netlink (route, uevent) | `curl https://…` and `ssh` work | 2–3 months |
| 7. Graphics and input | DRM/KMS on virtio-gpu (then simple-framebuffer, then AMD/Intel), evdev, xHCI + USB HID, `memfd`, dma-buf | SegLinux's Rust compositor and shell run unchanged on SegOS | 4–6 months |
| 8. Desktop apps | Sound (virtio-snd, Intel HDA → ALSA interface), `inotify`, `fanotify`, seccomp, namespaces (for Flatpak/Chromium sandbox), `io_uring` subset | Firefox, Chromium, LibreOffice, Discord run | 6+ months |
| 9. Games | Mesa radv/iris on real AMD/Intel GPUs (amdgpu-compatible DRM is the largest single task), `ptrace`, cgroups v2 subset, Wine/Proton | Steam installs; a Proton game runs | 1 year+ |
| 10. Release | Installer (reuse SegLinux Setup's design), updates, real-hardware testing, power management (ACPI S3, cpufreq) | Installable ISO | ongoing |

\* For one or two experienced developers. Honest expectation: a shell in
3–6 months, a desktop with a browser in 2–3 years, Steam games longer. GPU
drivers for real hardware are the hardest part, as they were for every other
new OS.

## 5. How compatibility is measured

- **Syscall conformance suite** (`userland/tests`): small programs whose
  output is recorded on real Linux and compared on SegOS. Plus imported
  suites: LTP (Linux Test Project), gVisor's syscall tests, musl's libc-test.
- **Syscall coverage table** generated from the source: implemented /
  partial / `ENOSYS` for all ~360 x86-64 syscalls, published in `docs/`.
- **Application ladder** run in CI nightly: BusyBox → bash → coreutils →
  python → gcc → curl → weston → SegLinux compositor → Firefox → Steam.
- **`strace` on Linux** to find which syscalls an application needs before
  working on it.

## 6. What SegLinux keeps

Deleting the tree is not throwing it away; the userland pieces come back on
top of SegOS once Phase 7 is reached, because they are Linux programs:

- `seglinux-compositor` (Smithay) and `seglinux-shell` — the desktop.
- `seglinux-installer` / `seglinux-setup` — the installer design.
- `seglinux-pe`, `segrun` — Windows program support via Wine.
- `docs/gaming.md`, tuning knowledge — scheduler and latency targets.

They stay available in git history (`git show main:crates/...`) and can be
restored into `userland/` when needed.

## 7. Risks

| Risk | Mitigation |
|---|---|
| Scope is enormous | Strict phase gates; every phase ends in a working demo; never start a phase before the previous one passes CI |
| Linux ABI has many undocumented behaviours | Compare against real Linux with the conformance suite; read Linux source for semantics (not copying code: GPL) |
| GPU drivers | Start with virtio-gpu and simple-framebuffer; AMD first on real hardware because its documentation and Mesa driver are open |
| Licensing | Kernel code original under MIT/Apache-2.0; no code copied from Linux (GPL-2.0); rootfs binaries keep their own licences |
| `unsafe` bugs | Confine to `hal` and audited modules, `#![forbid(unsafe_code)]` elsewhere, Miri for pure logic, KASAN-like checks in debug builds |

## 8. First concrete steps (Phase 0)

1. Delete the old tree on the working branch; add the new workspace,
   `rust-toolchain.toml` (stable, `rust-src`, `llvm-tools`), `xtask`.
2. Kernel entry via the `limine` crate; serial (COM1) logger; framebuffer
   text console.
3. GDT, TSS, IDT with handlers for all exceptions; double-fault stack.
4. Physical memory map from Limine → bitmap/buddy frame allocator;
   4-level paging with a higher-half direct map; `linked_list_allocator`
   heap, later own slab allocator.
5. `cargo xtask run` builds an ISO with Limine and boots it in QEMU with
   OVMF; `cargo xtask test` runs in-kernel tests and exits QEMU via
   `isa-debug-exit`.
6. GitHub Actions workflow: build, boot in QEMU, check serial output.
