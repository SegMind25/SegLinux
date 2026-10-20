#!/usr/bin/env python3
"""Boot an ISO headless in QEMU, log in on a text console and run seg.

Best effort and for the log only: it drives QEMU's monitor (screendump and
sendkey), reads the screen with tesseract and prints what it saw. It always
exits 0; the image checks before it are what fail the job.

usage: qemu_check.py ISO [boot_seconds]
"""
import os
import socket
import subprocess
import sys
import time

from PIL import Image

ISO = sys.argv[1]
BOOT_WAIT = int(sys.argv[2]) if len(sys.argv) > 2 else 240
MON = "/tmp/qemu-mon.sock"
WORK = "/tmp/qemu-check"
os.makedirs(WORK, exist_ok=True)

KEYS = {" ": "spc", "-": "minus", ".": "dot", ";": "semicolon", "/": "slash", "\n": "ret", "_": "shift-minus", "=": "equal"}


def log(msg):
    print(msg, flush=True)


class Monitor:
    def __init__(self, path):
        deadline = time.time() + 30
        while True:
            try:
                self.sock = socket.socket(socket.AF_UNIX)
                self.sock.connect(path)
                break
            except OSError:
                if time.time() > deadline:
                    raise
                time.sleep(0.5)
        self.sock.settimeout(5)
        self.read()

    def read(self):
        data = b""
        try:
            while True:
                chunk = self.sock.recv(4096)
                if not chunk:
                    break
                data += chunk
                if data.rstrip().endswith(b"(qemu)"):
                    break
        except socket.timeout:
            pass
        return data.decode(errors="replace")

    def cmd(self, text):
        self.sock.sendall(text.encode() + b"\n")
        return self.read()

    def key(self, name):
        self.cmd(f"sendkey {name}")
        time.sleep(0.12)

    def type(self, text):
        for ch in text:
            self.key(KEYS.get(ch, ch))

    def screen_text(self, tag):
        ppm = f"{WORK}/{tag}.ppm"
        self.cmd(f"screendump {ppm}")
        time.sleep(1)
        png = f"{WORK}/{tag}.png"
        try:
            img = Image.open(ppm)
            img = img.resize((img.width * 2, img.height * 2))
            img.save(png)
            out = subprocess.run(["tesseract", png, "-", "--psm", "6"], capture_output=True, text=True, timeout=60).stdout
        except Exception as e:  # the screen may not be written yet
            return f"(no screen: {e})"
        return "\n".join(line for line in out.splitlines() if line.strip())


def main():
    args = ["qemu-system-x86_64", "-m", "3072", "-smp", "2", "-cdrom", ISO, "-boot", "d",
            "-display", "none", "-vga", "std", "-monitor", f"unix:{MON},server,nowait",
            "-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0"]
    if os.access("/dev/kvm", os.R_OK | os.W_OK):
        args += ["-enable-kvm", "-cpu", "host"]
        log("qemu: using KVM")
    else:
        args += ["-cpu", "max"]
        log("qemu: no KVM, emulating (slow)")
    boot_wait = BOOT_WAIT if "-enable-kvm" in args else max(BOOT_WAIT, 420)
    qemu = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        mon = Monitor(MON)
        start = time.time()
        n = 0
        while time.time() - start < boot_wait:
            time.sleep(30)
            n += 1
            text = mon.screen_text(f"boot{n}")
            log(f"--- screen after {int(time.time() - start)} s:\n" + "\n".join(text.splitlines()[:12]))
        # Switch to the second console, where agetty waits for a login.
        mon.cmd("sendkey ctrl-alt-f2")
        time.sleep(5)
        log("--- console 2:\n" + mon.screen_text("tty2"))
        mon.type("seglinux\n")
        time.sleep(4)
        mon.type("seglinux\n")
        time.sleep(8)
        for i, command in enumerate(["seg info", "seg zram status", "seg tune", "seg svc list --enabled", "seg doctor"]):
            mon.type("clear\n")
            time.sleep(1)
            mon.type(command + "\n")
            time.sleep(12)
            log(f"--- $ {command}:\n" + mon.screen_text(f"cmd{i}"))
    except Exception as e:
        log(f"qemu check stopped: {type(e).__name__}: {e}")
    finally:
        qemu.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
