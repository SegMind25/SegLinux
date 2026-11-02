# Smithay 0.7.0, patched for SegLinux

The crates.io release of Smithay 0.7.0, with one addition: tearing (frames
shown at once instead of at the next vblank, for the lowest latency in
games), which Smithay does not offer yet.

- `DrmCompositor::set_tearing` / `tearing`: page flips of that screen ask
  for `DRM_MODE_PAGE_FLIP_ASYNC`.
- `DrmSurface::page_flip_tearing` (atomic and legacy): tries the flip with
  the async flag; when the kernel refuses (more than the picture changed,
  such as the pointer, or the driver cannot), the same frame waits for
  vblank as before.

- `IsAlive for SelectionSource`: the compositor notices when the
  application that copied is gone and offers its kept copy of the clipboard.
- `X11Wm::new_selection` flushes: X11 applications see at once that a
  Wayland application copied (before, only with the next X11 event).

Every change is marked `SegLinux` (`grep -rn SegLinux src`). To update
Smithay: copy the new release here and apply those changes again.
