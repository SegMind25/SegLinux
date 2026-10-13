# Updating SegLinux

An installed SegLinux keeps itself current with one command; there is no need
to download a new ISO for each release:

```sh
sudo seglinux-update
```

It updates, in one go:

* **every package of the system** (`pacman -Syu`): the kernel, drivers,
  Mesa, Wine, Steam, KDE applications, ... from Artix's repositories;
* **Brave**, the default browser, from [Chaotic-AUR](https://aur.chaotic.cx);
* **SegLinux itself**: the desktop, Start menu, Settings, Setup, `segrun`,
  the themes and default settings, from SegLinux's own package repository.
  Every ISO build publishes there the same `seglinux` package the image
  contains, so an installed system gets what a new ISO would bring;
* **new default applications**: when a newer SegLinux installs an
  application by default (such as Brave or kitty), it is offered once. One
  you removed on purpose is not installed again.

pacman lists what it will update and asks first; answer **Y**. To update
without questions (from a script, for instance):

```sh
sudo seglinux-update --noconfirm
```

Restart after an update that installed a new kernel or graphics driver.
`pacman -Q seglinux` shows the installed SegLinux version: `0.1.0.<number>`,
where the number grows with each release.

## Systems installed before `seglinux-update`

If the system answers `seglinux-update: command not found`, it was installed
from an older ISO. Get the command once, with:

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

* **"could not resolve host" or a download error**: check the connection
  (*Network* in the dock, or `nmtui`) and run the command again. If a mirror
  is mid-sync, a second try usually works.
* **"signature ... is unknown trust" or "invalid or corrupted package"**
  for Artix's packages: renew the keyrings first, then update again:

  ```sh
  sudo pacman -Sy artix-keyring archlinux-keyring
  sudo seglinux-update
  ```

* **"SegLinux's own updates are not published yet"**: SegLinux's repository
  has no release yet. Everything else was updated; try again later.
* **A file conflict** (`exists in filesystem`): a file was added by hand
  where a package now installs one. Move it away and update again.

Discover and plain `sudo pacman -Syu` work too once `seglinux-update` has run
once, because the repositories are then set up. Only `seglinux-update` also
offers new default applications.

## For maintainers: publishing updates

The **ISO** workflow publishes the update: after building, it signs the
`seglinux` package and uploads it, with the repository database and the
public key, to the release named `packages` ("SegLinux updates", marked as
a pre-release; the website ignores it because it holds no ISO). The package
version is `0.1.0.<number of commits>`, so each build is newer than the one
before. The workflow first uploads the package, then the database that names
it, then deletes the old packages. This way the database never names a
missing file.

It needs the signing key, as the Actions secret `SEGLINUX_SIGNING_KEY`.
Without it, builds still produce ISOs, but publish no update. Create the key
**once**, on any Linux system (or Git Bash or WSL on Windows):

```sh
export GNUPGHOME="$(mktemp -d)"
gpg --batch --passphrase '' --quick-gen-key "SegLinux Package Signing" ed25519 sign never
gpg --armor --export-secret-keys > seglinux-signing-key.asc
```

Then, on GitHub: the repository's **Settings → Secrets and variables →
Actions → New repository secret**, name `SEGLINUX_SIGNING_KEY`, and as the
value the whole content of `seglinux-signing-key.asc`, from
`-----BEGIN PGP PRIVATE KEY BLOCK-----` to the `END` line. (With the GitHub
CLI: `gh secret set SEGLINUX_SIGNING_KEY < seglinux-signing-key.asc`.)

Store `seglinux-signing-key.asc` somewhere safe and offline, then delete
the copy and the temporary `GNUPGHOME` folder. **Keep the same key from
then on.** Installed systems trust this key only. A new key would make
them refuse every later SegLinux update until each one trusts the new key
by hand. The key has no passphrase because the workflow uses it unattended;
GitHub's secret storage protects it, and it can sign nothing but SegLinux's
updates.

The next push whose commit message contains `[build iso]` (or a manual
**ISO** run with *release* checked) then builds the images and publishes
the first update. Release downloads, and so updates, need the repository to
be public, as the ISO downloads do.

How it fits together:

| Part | Where |
|---|---|
| The command | `data/system/seglinux-update`, installed as `/usr/bin/seglinux-update` |
| Applications offered once | `data/seglinux/default-apps` → `/usr/share/seglinux/default-apps` |
| SegLinux's public key | written by the workflow into `data/keyrings/`, installed to `/usr/share/pacman/keyrings/seglinux.gpg` and trusted by `pacman-key --populate seglinux` |
| Repository set up on install | Setup runs `seglinux-update --setup` in the new system |
| Signing and publishing | the *Publish the SegLinux update* step of `.github/workflows/iso.yml` |
| Chaotic-AUR | `iso/scripts/chaotic-aur.sh` (images and CI), `seglinux-update` (installed systems) |
