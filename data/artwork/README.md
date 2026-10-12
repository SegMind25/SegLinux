# SegLinux artwork

All of it is drawn from scratch for SegLinux, by code in this folder.

- `seglinux.svg`: the logo. An S of two arcs, cut diagonally into two
  segments, in a cyan-to-violet gradient on a dark rounded square. Installed
  as the `seglinux` icon (hicolor theme): the Start button, the Setup
  sidebar and `LOGO=` in os-release.
- `seglinux-dark.jpg`: the default wallpaper, the S in ribbons of light on
  deep navy, with the left half calm for desktop icons.
- `seglinux-flow.jpg`: a second wallpaper, a rose-to-violet sky with flowing
  ribbons over a blue floor.
- `../grub/seglinux/`: the boot menu theme, used on the install media and on
  installed systems: background with the logo and name, menu frames,
  highlight, scrollbar, the logo as menu icon, and fonts.

Both wallpapers are installed in `/usr/share/backgrounds/seglinux/`, the
boot theme in `/usr/share/grub/themes/seglinux/`.

## Drawing them again

Pillow and NumPy draw the images; `grub-mkfont` (GRUB) and the Inter font
make the boot menu fonts:

```sh
python3 data/artwork/make-brand.py      # seglinux-dark.jpg and the boot menu theme
python3 data/artwork/make-wallpaper.py  # seglinux-flow.jpg (2560 x 1440; WIDTH HEIGHT OUT for others)
```

`make-brand.py` draws the S from the same geometry as `seglinux.svg` (two
arcs of radius 19 in a 128-unit box, a 13-unit stroke, the cut from (40, 76)
to (88, 52)); change both together. The boot menu layout is in
`../grub/seglinux/theme.txt`.

The boot menu fonts (`seglinux-*.pf2`, named "SegLinux Sans") are bitmap
renderings of [Inter](https://rsms.me/inter/) by Rasmus Andersson, under the
SIL Open Font License 1.1.
