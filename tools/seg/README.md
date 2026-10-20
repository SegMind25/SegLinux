# seg

One small Rust program to look after a SegLinux system, the same on OpenRC,
runit, dinit and s6. No async runtime and one dependency
([lexopt](https://crates.io/crates/lexopt), a minimal argument parser); the
release binary is about 550 KiB.

```
seg info                      distro, kernel, init, uptime, memory, packages, disk
seg svc list|enable|disable|start|stop [name]
seg clean [--apply]           orphans, old cached packages, old logs
seg tune light|balanced|performance|apply|status
seg zram on|off|status
seg doctor                    look for problems and print their fixes
```

Commands that change the system need root and say so ("`seg tune
performance` needs root. Run it again with sudo"); errors are messages, not
panics. Reports (`info`, `svc list`, `tune status`, `zram status`, `doctor`,
`clean` without `--apply`) work as a normal user.

## seg info

```
SegLinux
  Kernel   7.2.11-zen1-1-zen
  Init     openrc
  Uptime   2h 15m
  Memory   1.9 GiB / 15.6 GiB (12%)
  Packages 912 (pacman)
  Disk /   21.4 GiB / 232.9 GiB (9%)
```

It reads /proc, /etc/os-release and pacman's local database directly; the only
program it starts is `df`.

## seg svc

One command for the four init systems, detected from PID 1 (`openrc-init`,
`runit`, `dinit`, `s6-svscan`), then from the state the init leaves in /run,
then from what is installed.

| | OpenRC | runit | dinit | s6 |
|---|---|---|---|---|
| `enable` | `/etc/runlevels/<level>/<name>` -> `/etc/init.d/<name>` | `/etc/runit/runsvdir/default/<name>` -> `/etc/runit/sv/<name>` | `/etc/dinit.d/boot.d/<name>` -> `../<name>` | `/etc/s6/adminsv/default/contents.d/<name>`, then `s6-db-reload` |
| `start`, `stop` | `rc-service` | `sv` | `dinitctl` | `s6-rc -u/-d change` |
| state in `list` | `rc-status -s` | `sv status` | `dinitctl list` | `s6-rc -a list` |

`enable` and `disable` only create or remove those links, so they also work in
a chroot or on a mounted system (`--offline` picks the init from what is
installed there instead of from the running PID 1). Service names are checked:
no paths, no hidden files.

## seg clean

Dry run unless `--apply` is given.

* **orphans**: `pacman -Qdtq`; applied with `pacman -Rns`.
* **package cache**: for every package and architecture, all but the newest 2
  versions (`--keep N`), with their `.sig` files. The versions are ordered
  with a port of libalpm's `vercmp` (epochs, `1.10 > 1.2`, `1.0a < 1.0`), so it
  keeps what `paccache -k2` would. `CacheDir` is read from pacman.conf.
  Partial downloads and anything that is not a finished package are left alone.
* **logs**: rotated logs (`messages.1`, `*.log.old`, `*-20240101.gz`) in
  /var/log older than 30 days (`--log-days N`). Live logs are never touched.
* **~/.cache**: its size and the five biggest folders (of the user who ran
  `sudo`). Reported, never deleted.

## seg tune

| | light | balanced | performance |
|---|---|---|---|
| `vm.swappiness` | 100 | 60 | 10 |
| `vm.vfs_cache_pressure` | 150 | 100 | 50 |
| `vm.dirty_ratio` | 10 | 20 | 15 |
| `vm.dirty_background_ratio` | 5 | 10 | 5 |
| CPU governor (first available) | powersave, conservative, schedutil, ondemand | schedutil, ondemand, powersave | performance |
| energy/performance preference | power | balance_performance | performance |

* With zram swap on, swappiness is at least 100: swapping to compressed memory
  is cheaper than re-reading files from disk.
* I/O scheduler of every disk: `none` for NVMe and virtual disks (their
  controller or host schedules), `mq-deadline` for SSDs, `bfq` for spinning
  disks. A scheduler the kernel does not have is reported, not forced.
* The profile is applied at once and saved: /etc/seg/tune.conf (the profile) and
  /etc/sysctl.d/99-seg.conf (the values, for the `sysctl` service). The
  `seg-tune` service runs `seg tune apply` after boot to set what only
  exists at run time (governor, schedulers).
* `power-profiles-daemon`, when it runs, also sets the CPU's energy preference;
  `seg-tune` starts after it, so the saved profile wins at boot and the last
  change wins afterwards.
* `--dry-run` shows the changes without making them.

## seg zram

`seg zram on` loads the module if needed and makes /dev/zram0 swap: zstd,
half the RAM (at most 8 GiB, because the device's table uses memory in
proportion to its size; `--size 2G` overrides), priority 100. It also turns
zswap off (it would compress the same pages twice) and sets
`vm.page-cluster=0`. `seg zram off` swaps off and frees the memory; `status`
shows size, use and compression ratio. The `seg-zram` service runs it at boot.

## seg doctor

| check | looks at | fix it prints |
|---|---|---|
| services | enabled services that are stopped or crashed | the init's start command, or `seg svc disable` |
| firmware | `dmesg`: "Direct firmware load for ... failed with error -2" | `pacman -F` to find the package, then install and reboot |
| disk | /, /home, /var, /usr under 1 GiB free or 90 % full; /boot under 20 MiB | `seg clean --apply` |
| pacman | /var/lib/pacman/db.lck with no pacman running | `sudo rm /var/lib/pacman/db.lck` |
| keyring | /etc/pacman.d/gnupg, `artix-keyring`, Artix (and Arch, if enabled) keys | `pacman-key --init` / `--populate artix` |

Exit status 1 when it found a problem. The kernel log and the keyring need
root to read (`sudo seg doctor`); without it those checks say "skip".
Some drivers (iwlwifi, amdgpu) ask for a newer firmware file first and fall
back, which also appears in the log; the report says so.

## Shipped in the package (`pkg/segctl`)

* `/usr/bin/seg`
* `seg-tune` and `seg-zram` for OpenRC (`/etc/init.d`), runit
  (`/etc/runit/sv`) and dinit (`/etc/dinit.d`); the package enables both
  (`seg svc --offline enable`) when it is installed, also in the ISO build's chroot
* `/etc/seg/tune.conf` and `/etc/sysctl.d/99-seg.conf`: the balanced profile
* `/usr/lib/seg/fstrim-weekly` and `/etc/cron.d/seg-fstrim`: cron checks every
  hour and trims once a week (a stamp in /var/lib/seg), so a computer that
  was off on the usual day catches up

## Building

```sh
cd tools/seg
cargo fmt --check && cargo clippy -- -D warnings && cargo test
cargo build --release          # target/release/seg
make -C ../../pkg/segctl package   # the pacman package, on Arch/Artix
```

The release profile is `opt-level = "z"`, LTO, one codegen unit,
`panic = "abort"`, stripped. Outside tests, `unwrap`, `expect`, `panic!` and
slice indexing are compile errors (`#![deny(clippy::...)]`), and output goes
through a writer that ends quietly on a closed pipe (`seg clean | head`).

The code is split so that every parser is a pure function with tests
(`/proc/meminfo`, `df -Pk`, `rc-status`, `sv status`, `dinitctl list`,
`/proc/swaps`, `dmesg`, pacman cache file names, version order), and the
parts that write to /sys, /proc and /etc take a root directory, which the
tests point at a fake tree.
