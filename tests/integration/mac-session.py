#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the shared Linux process kernel through the signed HVF product."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import termios
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
parser.add_argument('--rootfs', type=Path, required=True)
parser.add_argument('--fixture', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
runner, rootfs = str(args.runner.resolve()), str(args.rootfs.resolve())
rows = []

def run(name, command, expected, options=()):
    result = subprocess.run([runner, '--timeout', '30', '--network', 'off', *options, *command],
                            capture_output=True, timeout=35)
    (args.output / (name + '.stdout')).write_bytes(result.stdout)
    (args.output / (name + '.stderr')).write_bytes(result.stderr)
    assert result.returncode == 0, (name, result.returncode, result.stderr)
    assert result.stdout == expected, (name, result.stdout, expected)
    rows.append(dict(name=name, passed=True))

try:
    run('process-threads', [str(args.fixture.resolve())],
        b'fork-cow-pipe-wait: ok\nthreads-tls-futex-atomics: ok\nstop-continue-wait: ok\n')
    run('shell-processes', ['/bin/sh', '-c', '''set -eu
/usr/bin/test -x /bin/ls
printf 'alpha\nbeta\n' | /bin/grep beta | /usr/bin/wc -l
/bin/sh -c 'exit 7' || test "$?" = 7
/bin/sleep 0.02 & child=$!; wait "$child"
i=0; while test "$i" -lt 30; do /bin/busybox true; i=$((i+1)); done
echo external-exec-pipe-wait-ok'''], b'1\nexternal-exec-pipe-wait-ok\n', ['--rootfs', rootfs])
    script = args.output / 'script.sh'
    script.write_text('#!/bin/sh\n/bin/echo shebang-exec-ok\n')
    run('script', ['/bin/sh', '-c', 'exec /tmp/script.sh'], b'shebang-exec-ok\n',
        ['--rootfs', rootfs, '--exec-file', '/tmp/script.sh=' + str(script.resolve())])
    store = str((args.output / 'storage').resolve())
    run('storage-write', ['/bin/sh', '-c', 'echo persisted > /root/session-check'], b'',
        ['--rootfs', rootfs, '--storage', store])
    run('storage-read', ['/bin/cat', '/root/session-check'], b'persisted\n', ['--storage', store])

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 33, 101, 0, 0))
    original = termios.tcgetattr(slave)
    process = subprocess.Popen([runner, '--rootfs', rootfs, '--network', 'off', '--timeout', '30', '--pty', '/bin/sh', '-i'],
                               stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    transcript = bytearray()
    cursor = 0
    def expect(needle, timeout=5):
        global cursor
        deadline = time.monotonic() + timeout
        start = cursor
        while needle not in transcript[start:]:
            assert time.monotonic() < deadline, (needle, bytes(transcript))
            if select.select([master], [], [], 0.1)[0]:
                transcript.extend(os.read(master, 65536))
        cursor = transcript.index(needle, start) + len(needle)
        return bytes(transcript[start:cursor])
    try:
        expect(b'/ # ')
        assert b"job control turned off" not in transcript
        os.write(master, b'/bin/stty size\n')
        expect(b'33 101')
        expect(b'/ # ')
        os.write(master, b'/bin/sleep 30\n')
        time.sleep(0.15)
        os.write(master, b'\x03')
        expect(b'/ # ')
        os.write(master, b'/bin/echo interrupt:$?\n')
        expect(b'interrupt:130')
        expect(b'/ # ')
        os.write(master, b'/bin/sleep 30\n')
        time.sleep(0.15)
        os.write(master, b'\x1a')
        expect(b'Stopped')
        expect(b'/ # ')
        os.write(master, b'fg\n')
        time.sleep(0.15)
        os.write(master, b'\x03')
        expect(b'/ # ')
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 44, 120, 0, 0))
        os.write(master, b'/bin/stty size\n')
        expect(b'44 120')
        expect(b'/ # ')
        os.write(master, b'exit\n')
        expect(b"exit\r\n")
        assert process.wait(timeout=5) == 0
        restored = termios.tcgetattr(slave)
        # Darwin sets PENDIN itself when switching back to canonical input.
        # It is a transient driver state, not a user terminal configuration.
        restored[3] &= ~termios.PENDIN
        original[3] &= ~termios.PENDIN
        assert restored == original, ('host terminal was not restored', original, restored)
        rows.append(dict(name='interactive-job-control-resize-restore', passed=True))
    finally:
        (args.output / 'terminal.log').write_bytes(transcript)
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)
finally:
    (args.output / 'results.json').write_text(json.dumps(dict(passed=len(rows) == 6, rows=rows), indent=2) + '\n')
print(json.dumps(dict(passed=True, rows=rows)))
