#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Validate Linux vDSO clocks and futex handoffs on a signed Mac executor."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
parser.add_argument('--clock', type=Path, required=True, help='Static-musl linux-clock.c ELF')
parser.add_argument('--ring', type=Path, required=True, help='Static-musl futex-ring.c ELF')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
report = dict(passed=False, rows=[], sha256={name: hashlib.sha256(path.read_bytes()).hexdigest()
              for name, path in [('runner', args.runner), ('clock', args.clock), ('ring', args.ring)]})


def run(name, cpus, fixture, arguments, metrics=False):
    command = [str(args.runner.resolve()), '--no-pty', '--no-windows', '--network', 'off',
               '--cpus', str(cpus), '--memory-mib', 'auto', '--timeout', '20']
    if metrics:
        command.append('--metrics')
    result = subprocess.run(command + [str(fixture.resolve()), *arguments],
                            capture_output=True, timeout=25,
                            env={**os.environ, 'TRAMES_MAC_BUNDLED': '1'})
    (args.output / (name + '.stdout')).write_bytes(result.stdout)
    (args.output / (name + '.stderr')).write_bytes(result.stderr)
    row = dict(name=name, cpus=cpus, exit=result.returncode, passed=False)
    report['rows'].append(row)
    assert result.returncode == 0, (name, result.stdout, result.stderr)
    return row, result


try:
    expected = (b'clock-exec-ok\nclock: ELF/versioned symbols, libc, syscall agreement, '
                b'threads, fork/exec passed\n')
    for cpus in [1, 12]:
        row, result = run(f'clock-{cpus}', cpus, args.clock, [])
        assert result.stdout == expected and not result.stderr, result
        row['passed'] = True

        # Counters prove libc uses the fast path. This is not a timing gate.
        row, result = run(f'clock-counters-{cpus}', cpus, args.clock, ['--bench', '200000'], True)
        row['work'] = json.loads(result.stdout)
        prefix = b'trames mac metrics: '
        metrics = next(json.loads(line[len(prefix):]) for line in result.stderr.splitlines()
                       if line.startswith(prefix))
        row['syscalls'] = metrics['syscalls']
        assert row['work']['vdso'] and row['work']['calls'] == 200000
        assert row['syscalls'] < 32, row
        row['passed'] = True
    for cpus, threads in [(1, 2), (6, 6), (12, 12)]:
        row, result = run(f'ring-{cpus}', cpus, args.ring, [str(threads), '1000'])
        row['work'] = json.loads(result.stdout)
        assert not result.stderr and row['work']['threads'] == threads
        assert row['work']['rounds'] == 1000 and row['work']['checksum'] == threads * 1000
        row['passed'] = True
    report['passed'] = True
    print('PASS: Linux clock ABI, fast-path syscall counters, and futex handoffs')
finally:
    (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
