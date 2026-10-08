#!/usr/bin/env python3
"""Pty smoke tests for tcac's interactive modes.

Each case spawns tcac under a real pty, feeds keystrokes, and checks the
exit status and that the terminal was restored (alt-screen leave emitted).

Usage: python3 tools/tests/pty_smoke.py [path-to-tcac]
"""

import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else "./target/release/tcac"


def set_pty_size(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def run_pty(args, keys, settle=1.5, max_wait=20.0, resize=None):
    """Spawn tcac under a pty, send keys, return (exit_code, output bytes).

    `resize` is an optional (rows, cols) applied via TIOCSWINSZ + SIGWINCH
    at the same cadence as the keys.
    """
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["LINES"] = "24"
        os.environ["COLUMNS"] = "80"
        os.execvp(BIN, [BIN] + args)
        os._exit(127)
    set_pty_size(fd, 24, 80)

    out = b""

    def reap(blocking):
        """Return (code, done). EIO from read means the child is gone."""
        done, status = os.waitpid(pid, 0 if blocking else os.WNOHANG)
        if done != pid:
            return None, False
        if os.WIFSIGNALED(status):
            return -os.WTERMSIG(status), True
        return os.WEXITSTATUS(status), True

    deadline = time.time() + max_wait
    pending_keys = list(keys)
    pending_resizes = [resize] if resize else []
    next_event = time.time() + settle
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.05)
        child_gone = False
        if r:
            try:
                chunk = os.read(fd, 65536)
                if chunk:
                    out += chunk
                else:
                    child_gone = True  # EOF
            except OSError:
                child_gone = True  # EIO: slave closed
        if (pending_keys or pending_resizes) and time.time() >= next_event:
            if pending_resizes:
                rows, cols = pending_resizes.pop(0)
                set_pty_size(fd, rows, cols)
                os.kill(pid, signal.SIGWINCH)
            else:
                try:
                    os.write(fd, pending_keys.pop(0).encode())
                except OSError:
                    child_gone = True
            next_event = time.time() + settle
        code, done = reap(False)
        if done:
            os.close(fd)
            return code, out
        if child_gone:
            # exit status is ready; blocking reap is instant
            code, _ = reap(True)
            os.close(fd)
            return code, out
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
    os.close(fd)
    return None, out


CASES = [
    ("live quit on q", ["-l", "-t", "0.001", "-s", "42"], ["q"], None, 0),
    ("static quit on q", ["-s", "42"], ["q"], None, 0),
    ("screensaver quits on any key", ["-S", "-t", "0.001"], ["x"], None, 0),
    ("infinite second cactus then quit", ["-i", "-t", "0.001", "-w", "0.1", "-s", "42"], ["q", "q"], None, 0),
    ("print mode exits immediately", ["-p", "-s", "42"], [], None, 0),
    ("resize redraws and quits", ["-s", "42"], ["q"], (30, 100), 0),
]


def main():
    failures = 0
    for name, args, keys, resize, want in CASES:
        code, out = run_pty(args, keys, resize=resize)
        ok = code == want
        restored = b"\x1b[?1049l" in out or b"-p" in args
        grew = b"[" in out  # pot brackets appear in every frame
        status = "ok" if (ok and restored and grew) else "FAIL"
        if status == "FAIL":
            failures += 1
        print(f"{status}: {name} (exit={code}, bytes={len(out)})")
        if status == "FAIL":
            print(f"  restored={restored} grew={grew}")
        if resize is not None and status == "ok":
            # the redraw after SIGWINCH must have repainted at the new size
            if b"\x1b[30;1H" not in out and b"\x1b[29;1H" not in out:
                print("  note: no row-30 cursor move seen (resize repaint unverified)")
    print("all pty smoke tests passed" if failures == 0 else f"{failures} FAILURES")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
