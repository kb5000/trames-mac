#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Require real presentation callbacks, repeated native windows, and a live CLI.

Run on an unlocked Apple Silicon desktop. --client is the unchanged Linux
AArch64 wayland-client.c fixture linked with original libwayland-client;
--libraries contains its original distro libraries at their guest paths.
This test does not synthesize host UI events or substitute frame callbacks.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
parser.add_argument('--rootfs', type=Path, required=True)
parser.add_argument('--client', type=Path, required=True)
parser.add_argument('--libraries', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
runner = str(args.runner.resolve())
base = [runner, '--rootfs', str(args.rootfs.resolve()), '--network', 'off',
        '--cpus', '4', '--no-pty', '--timeout', '90']
rows = []
transcript = bytearray()
process = None
try:
    # Both modes must leave AppKit uninitialized for a console-only process.
    for flags in ([], ['--no-windows']):
        result = subprocess.run(base + flags + ['/bin/echo', 'console-ok'],
                                env=dict(os.environ, TRAMES_MAC_WINDOW_TRACE='1'),
                                capture_output=True, timeout=30)
        assert result.returncode == 0, result.stderr
        assert result.stdout == b'console-ok\n', result.stdout
        assert b'AppKit' not in result.stderr, result.stderr
        assert (b'main run-loop source created' in result.stderr) == (not flags), result.stderr
        rows.append(dict(name='console' + ('-disabled' if flags else '-automatic'), passed=True))
    command = ('tar -xf /tmp/libs.tar -C /; '
               'for cycle in 1 2 3; do /tmp/client 120; echo cycle:$cycle; sleep 1; done; '
               'echo TERMINAL_SESSION_STILL_ALIVE; read reply; '
               '[ "$reply" = resume ]; echo TERMINAL_INPUT_STILL_ALIVE')
    process = subprocess.Popen(base + [
        '--exec-file', '/tmp/client=' + str(args.client.resolve()),
        '--exec-file', '/tmp/libs.tar=' + str(args.libraries.resolve()),
        '/bin/sh', '-ec', command], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, env=dict(os.environ, TRAMES_MAC_WINDOW_TRACE='1'))
    deadline = time.monotonic() + 100
    while b'TERMINAL_SESSION_STILL_ALIVE\n' not in transcript:
        assert time.monotonic() < deadline, bytes(transcript)
        if select.select([process.stdout], [], [], .25)[0]:
            chunk = os.read(process.stdout.fileno(), 65536)
            assert chunk, (process.poll(), bytes(transcript))
            transcript.extend(chunk)
    assert process.poll() is None, bytes(transcript)
    process.stdin.write(b'resume\n')
    process.stdin.flush()
    tail, _ = process.communicate(timeout=10)
    transcript.extend(tail)
    assert process.returncode == 0, bytes(transcript)
    assert transcript.count(b'wayland-client-ok windows=2 frames=120') == 3, bytes(transcript)
    assert transcript.count(b'trames mac window created:') == 6, bytes(transcript)
    assert transcript.count(b'trames mac windows: AppKit ready') == 1, bytes(transcript)
    assert b'TERMINAL_INPUT_STILL_ALIVE\n' in transcript, bytes(transcript)
    assert b'GPU display command failed' not in transcript, bytes(transcript)
    assert b'window error:' not in transcript, bytes(transcript)
    rows.append(dict(name='six-windows-three-cycles-real-frame-callbacks-cli-survives', passed=True))
finally:
    if process is not None and process.poll() is None:
        process.kill()
        transcript.extend(process.communicate(timeout=10)[0])
    (args.output / 'windows.log').write_bytes(transcript)
    hashes = {name + '_sha256': hashlib.sha256(getattr(args, name).read_bytes()).hexdigest()
              for name in ('runner', 'rootfs', 'client', 'libraries')}
    (args.output / 'results.json').write_text(json.dumps(dict(passed=len(rows) == 3,
        rows=rows, **hashes), indent=2) + '\n')
print(json.dumps(dict(passed=True, rows=rows)))
