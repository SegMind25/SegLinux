# Updating SegLinux

An installed SegLinux keeps itself current with one command. There is no
need to download a new ISO for each release. Open a terminal (Super+Return)
and type:

```sh
sudo seglinux-update
```

(The command is `seglinux-update`, one word with a dash, after `sudo`.)

It updates, in one go:

* **every package of the system** (`pacman -Syu`): the kernel, drivers,
  Mesa, Wine, Steam, KDE applications, ... from Artix's repositories;
* **Brave**, the default browser, from [Chaotic-AUR](https://aur.chaotic.cx);
* **SegLinux itself**: the desktop, Start menu, Settings, the sign-in screen,
  Setup, `segrun`, the themes and default settings. Every change to SegLinux
  is published as an update, signed, so an installed system gets what a new
  ISO would bring, without the ISO;
* **new default applications**: when a newer SegLinux installs an
  application by default (such as Brave or kitty), it is offered once. One
  you removed on purpose is not installed again;
* **the new sign-in screen**, on systems installed when SegLinux still used
  SDDM: it asks once whether to switch (it appears at the next restart).

pacman lists what it will update and asks first; answer **Y**. To update
without questions (from a script, for instance):

```sh
sudo seglinux-update --noconfirm
```

Restart after an update that installed a new kernel or graphics driver.
`pacman -Q seglinux` shows the installed SegLinux version: `0.1.0.<number>`,
where the number grows with each release; the command shows it at the end.

## "seglinux-update: command not found"

The system was installed from an ISO older than this command (before 13
October 2026). Get it once with these two lines:

```sh
curl -fsSL https://raw.githubusercontent.com/SegMind25/SegLinux/main/data/system/seglinux-update -o /tmp/seglinux-update
sudo bash /tmp/seglinux-update
```

This sets up Chaotic-AUR and SegLinux's repository, then updates everything,
including the `seglinux` package, which installs `seglinux-update` itself.
From then on `sudo seglinux-update` is enough.

## Is it safe?

pacman checks the signature of every package before installing it:

* Artix's and Chaotic-AUR's packages are signed with their keys, as on any
  Artix system;
* SegLinux's packages are signed with **SegLinux's key**, which comes with
  SegLinux: images include it, and the `seglinux` package carries it. Systems
  older than signed updates take it once from the same place as the updates,
  over HTTPS.

The SegLinux repository is set up with `SigLevel = Required`. A package
that is unsigned or signed with any other key is refused, so neither a
tampered download nor a different publisher can slip in an update.
(This was tested with packages signed by SegLinux's key, by another key and
not signed at all.)

`seglinux-update` changes `/etc/pacman.conf` only to add the two repositories,
each marked `# Added by seglinux-update.`. It keeps the previous file as
`/etc/pacman.conf.before-seglinux-update`.

## When an update fails

* **"no internet connection"** or a download error: connect (*Network* in
  the dock, or `nmtui`) and run the command again. If a mirror is mid-sync,
  a second try usually works.
* **"pacman is busy"**: another update, or Discover, is running. Wait for it
  to finish. If nothing is running (the computer was switched off during an
  update), remove the stale lock: `sudo rm /var/lib/pacman/db.lck`.
* **"signature ... is unknown trust" or "invalid or corrupted package"**
  for Artix's packages: renew the keyrings first, then update again:

  ```sh
  sudo pacman -Sy artix-keyring archlinux-keyring
  sudo seglinux-update
  ```

* **"SegLinux's own updates are not published yet"**: no SegLinux update has
  been published so far (see *For maintainers* below). Everything else was
  updated.
* **"Chaotic-AUR could not be set up"**: its key server or mirror did not
  answer. Everything else was updated; Brave is updated next time.
* **A file conflict** (`exists in filesystem`): a file was added by hand
  where a package now installs one. Move it away and update again.

Discover and plain `sudo pacman -Syu` work too once `seglinux-update` has run
once, because the repositories are then set up. Only `seglinux-update` also
offers new default applications and the new sign-in screen.

## For maintainers: publishing updates

The **SegLinux update** workflow (`.github/workflows/update.yml`) publishes
the updates. On every push to `main` that changes SegLinux (its crates,
`data/`, the packaging), it builds the `seglinux` package (no ISO), signs it
and uploads it, with the repository database and the public key, to the
release named `packages` ("SegLinux updates", a pre-release; the website
ignores it because it holds no ISO). *Actions → SegLinux update → Run
workflow* publishes the current `main` by hand. The package version is
`0.1.0.<number of commits>`, so each update is newer than the one before.
The package is uploaded first, then the database that names it, then the old
packages are deleted, so the database never names a missing file.

### The signing key (once)

Updates are only published once the repository has the signing key, as the
Actions secret `SEGLINUX_SIGNING_KEY`. Until then each run ends with the
warning *No SegLinux update published*, and `seglinux-update` says SegLinux's
updates are not published yet. To create the key, run this once on any
Linux with gpg, a SegLinux install included (as yourself, not root):

```sh
curl -fsSL https://raw.githubusercontent.com/SegMind25/SegLinux/main/packaging/make-signing-key.sh | bash
```

It writes `~/seglinux-signing-key.asc` and opens GitHub's page for a new
secret (or adds the secret itself when the GitHub CLI is signed in). There:

1. **Name**: `SEGLINUX_SIGNING_KEY`
2. **Secret**: the whole content of `~/seglinux-signing-key.asc`
   (`cat ~/seglinux-signing-key.asc`), from
   `-----BEGIN PGP PRIVATE KEY BLOCK-----` to the `END` line.
3. **Add secret**, then *Actions → SegLinux update → Run workflow*.

That publishes the first update; installed systems get it with
`sudo seglinux-update`. (Without SegLinux, in Git Bash or WSL on Windows:
`packaging/make-signing-key.sh` from a clone does the same.)

Store `seglinux-signing-key.asc` somewhere safe and offline. **Keep the same
key from then on.** Installed systems trust this key only. A new key would
make them refuse every later SegLinux update until each one trusts the new
key by hand. The key has no passphrase because the workflow uses it
unattended; GitHub's secret storage protects it, and it signs nothing but
SegLinux's updates. ISO builds also put its public half into the image, so
newly installed systems trust it from the start.

How it fits together:

| Part | Where |
|---|---|
| The command | `data/system/seglinux-update`, installed as `/usr/bin/seglinux-update` |
| Applications offered once | `data/seglinux/default-apps` → `/usr/share/seglinux/default-apps` |
| SegLinux's public key | written by `iso/scripts/signing-key.sh` into `data/keyrings/`, installed to `/usr/share/pacman/keyrings/seglinux.gpg` and trusted by `pacman-key --populate seglinux` |
| Repository set up on install | Setup runs `seglinux-update --setup` in the new system |
| Building, signing and publishing | `.github/workflows/update.yml`: `iso/build.sh --package-only`, `iso/scripts/publish-update.sh` |
| Creating the key | `packaging/make-signing-key.sh` |
| Chaotic-AUR | `iso/scripts/chaotic-aur.sh` (images and CI), `seglinux-update` (installed systems) |
| Tests | the *sudo seglinux-update (Artix)* job of `.github/workflows/ci.yml` |
