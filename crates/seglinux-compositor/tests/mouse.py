#!/usr/bin/env python3
"""A virtual mouse (uinput, absolute like QEMU's tablet) that plays a script
of moves and clicks on a 1024x768 screen and photographs it with grim.

    mouse.py OUT_DIR  (inside the drm-vm.sh virtual machine)

Steps, for the first terminal (placed at 162,149, 700x500 under a 30 px
title bar): drag it by its title bar to the top edge (a photo of the snap
preview), let go (maximized), double-click its title bar (its size back),
drag it into the top-left corner (a quarter), then pull its right border
(wider). Each photo includes the pointer.
"""
import fcntl
import os
import struct
import subprocess
import sys
import time

EV_SYN, EV_KEY, EV_ABS = 0, 1, 3
SYN_REPORT = 0
BTN_LEFT, BTN_RIGHT = 0x110, 0x111
ABS_X, ABS_Y = 0, 1
UI_SET_EVBIT, UI_SET_KEYBIT, UI_SET_ABSBIT = 0x40045564, 0x40045565, 0x40045567
UI_DEV_CREATE, UI_DEV_DESTROY = 0x5501, 0x5502
MAX = 32767
W, H = 1024, 768

out = sys.argv[1]
fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
fcntl.ioctl(fd, UI_SET_EVBIT, EV_ABS)
fcntl.ioctl(fd, UI_SET_EVBIT, EV_SYN)
fcntl.ioctl(fd, UI_SET_KEYBIT, BTN_LEFT)
fcntl.ioctl(fd, UI_SET_KEYBIT, BTN_RIGHT)
fcntl.ioctl(fd, UI_SET_ABSBIT, ABS_X)
fcntl.ioctl(fd, UI_SET_ABSBIT, ABS_Y)
absmax = [0] * 64
absmax[ABS_X] = absmax[ABS_Y] = MAX
device = struct.pack(
    "80sHHHHi" + "64i" * 4,
    b"SegLinux test mouse",
    0x03, 0x1234, 0x5678, 1,  # USB
    0,
    *absmax, *([0] * 64), *([0] * 64), *([0] * 64),
)
os.write(fd, device)
fcntl.ioctl(fd, UI_DEV_CREATE)
time.sleep(4)  # udev and libinput find it


def emit(kind, code, value):
    os.write(fd, struct.pack("qqHHi", 0, 0, kind, code, value))


def sync():
    emit(EV_SYN, SYN_REPORT, 0)
    time.sleep(0.03)


def move(x, y):
    emit(EV_ABS, ABS_X, round(x * MAX / (W - 1)))
    emit(EV_ABS, ABS_Y, round(y * MAX / (H - 1)))
    sync()


def glide(a, b, steps=20):
    for i in range(1, steps + 1):
        move(a[0] + (b[0] - a[0]) * i / steps, a[1] + (b[1] - a[1]) * i / steps)


def button(down):
    emit(EV_KEY, BTN_LEFT, 1 if down else 0)
    sync()
    # As a hand does: libinput may hold a press back briefly (debouncing),
    # and moving at once would land it a step further.
    time.sleep(0.3)


def photo(name):
    time.sleep(1.5)
    ok = subprocess.run(["grim", "-c", f"{out}/{name}.png"]).returncode == 0
    print(f"{name}-ok" if ok else f"{name}-failed", flush=True)


bar = (400, 134)
move(*bar)
time.sleep(0.5)
button(True)
glide(bar, (500, 300))
glide((500, 300), (512, 1))
photo("mouse-preview")
button(False)
photo("mouse-maximized")
# The maximized window's title bar is at the top: double-click it.
move(400, 15)
emit(EV_KEY, BTN_LEFT, 1)
sync()
emit(EV_KEY, BTN_LEFT, 0)
sync()
time.sleep(0.1)
emit(EV_KEY, BTN_LEFT, 1)
sync()
emit(EV_KEY, BTN_LEFT, 0)
sync()
photo("mouse-restored")
# Wherever it is now, its title bar: from the double-click's place.
glide((400, 15), (400, 134))
time.sleep(0.5)
button(True)
glide((400, 134), (300, 200))
glide((300, 200), (0, 5))
button(False)
photo("mouse-quarter")
# The quarter is 512 wide: its right border just beyond.
move(514, 200)
photo("mouse-border")
button(True)
glide((514, 200), (760, 200))
button(False)
photo("mouse-resized")

fcntl.ioctl(fd, UI_DEV_DESTROY)
os.close(fd)
