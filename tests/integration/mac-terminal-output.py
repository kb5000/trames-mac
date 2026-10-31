#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Verify host diagnostics and guest termios through a real terminal device."""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import termios
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--rootfs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    base = [str(args.runner.resolve()), '--rootfs', str(args.rootfs.resolve()),
            '--network', 'off', '--no-windows', '--timeout', '15']
    rows = []
    for metrics in (False, True):
        master, slave = pty.openpty()
        original = termios.tcgetattr(slave)
        script = "printf 'normal-first\\nnormal-second\\n'; /bin/stty -opost; printf 'raw-first\\nraw-second\\n'; /bin/stty opost; printf 'normal-restored\\n'"
        command = base + (['--metrics'] if metrics else []) + ['--pty', '/bin/sh', '-ec', script]
        child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        transcript = bytearray()
        deadline = time.monotonic() + 20
        try:
            while time.monotonic() < deadline:
                if select.select([master], [], [], .1)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            break
                        raise
                    if not chunk:
                        break
                    transcript.extend(chunk)
                elif child.poll() is not None:
                    break
            assert child.wait(timeout=2) == 0
            data = bytes(transcript)
            (args.output / ('metrics.pty' if metrics else 'normal.pty')).write_bytes(data)
            assert b'trames: session ready\r\n' in data, data
            assert b'normal-first\r\nnormal-second\r\n' in data, data
            assert b'raw-first\nraw-second\n' in data, data
            assert b'normal-restored\r\n' in data, data
            if metrics:
                host = data[data.index(b'trames mac parallel:'):]
                assert host and host.endswith(b'\r\n')
                assert all(host[i - 1:i] == b'\r' for i, b in enumerate(host) if b == 10), host
            restored = termios.tcgetattr(slave)
            original[3] &= ~termios.PENDIN
            restored[3] &= ~termios.PENDIN
            assert restored == original
            rows.append({'case': 'PTY with metrics' if metrics else 'PTY', 'passed': True})
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
            os.close(slave)
    result = subprocess.run(base + ['--no-pty', '/bin/sh', '-ec', "printf 'redirected\\noutput\\n'"], capture_output=True, timeout=20)
    assert result.returncode == 0, result.stderr
    assert result.stdout == b'redirected\noutput\n', result.stdout
    assert result.stderr == b'trames: session ready\n', result.stderr
    rows.append({'case': 'redirected output', 'passed': True})
    (args.output / 'results.json').write_text(json.dumps({'runner_sha256': hashlib.sha256(args.runner.read_bytes()).hexdigest(), 'cases': rows}, indent=2) + '\n')
    print('3 terminal output cases passed')


if __name__ == '__main__':
    main()
