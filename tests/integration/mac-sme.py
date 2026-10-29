#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the real Linux SME ABI on a supported, signed Mac executor."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
parser.add_argument('--fixture', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
expected = b'SME: VL, exec, fork, CLONE_VM, streaming signal frame, ZA/ZT/TLS restoration passed\n'
report = dict(passed=False,
              runner_sha256=hashlib.sha256(args.runner.read_bytes()).hexdigest(),
              fixture_sha256=hashlib.sha256(args.fixture.read_bytes()).hexdigest(), rows=[])
try:
    for cpus, memory in [(1, '4096'), (12, 'auto')]:
        name = f'{cpus}cpu-{memory}'
        result = subprocess.run([str(args.runner.resolve()), '--no-pty', '--no-windows',
                                 '--network', 'off', '--cpus', str(cpus), '--memory-mib', memory,
                                 '--timeout', '20', str(args.fixture.resolve())],
                                capture_output=True, timeout=25,
                                env={**os.environ, 'TRAMES_MAC_BUNDLED': '1'})
        (args.output / (name + '.stdout')).write_bytes(result.stdout)
        (args.output / (name + '.stderr')).write_bytes(result.stderr)
        row = dict(cpus=cpus, memory_mib=memory, exit=result.returncode,
                   passed=(result.returncode, result.stdout, result.stderr) == (0, expected, b''))
        report['rows'].append(row)
        assert row['passed'], (row, result.stdout, result.stderr)
        print('PASS', name, flush=True)
    report['passed'] = True
finally:
    (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
