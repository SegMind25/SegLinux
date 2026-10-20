# The seg tool

`seg` looks after a SegLinux system: `seg info`, `seg svc` (services on
OpenRC, runit, dinit or s6), `seg clean`, `seg tune`, `seg zram`, `seg game`
(boost while a game runs, a readiness check, the right driver packages) and
`seg doctor`. It is a small Rust program in `tools/seg` and is installed from
the `segctl` package. For games, see [gaming.md](gaming.md#seg-game).

Everything about it is in [tools/seg/README.md](../tools/seg/README.md).
