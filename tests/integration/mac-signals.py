#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare real hardware signal recovery with native Linux, using identical ELFs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
parser.add_argument('--fixtures', type=Path, required=True,
                    help='Original static-musl ELFs and NAME.stdout from native Linux')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
report = dict(passed=False, runner_sha256=hashlib.sha256(args.runner.read_bytes()).hexdigest(), rows=[])
try:
    for name, mode in [('cpu-signals', []), ('signal-leaf-stack', []),
                       ('task-signals', []), ('task-signals', ['autodisarm'])]:
        stem = name + ('-auto' if mode else '')
        fixture = args.fixtures / name
        expected = (args.fixtures / (stem + '.stdout')).read_bytes()
        assert expected and not (args.fixtures / (stem + '.stderr')).read_bytes()
        actual = subprocess.run([str(args.runner.resolve()), '--timeout', '20',
                                 str(fixture.resolve()), *mode], capture_output=True, timeout=25)
        (args.output / (stem + '.stdout')).write_bytes(actual.stdout)
        (args.output / (stem + '.stderr')).write_bytes(actual.stderr)
        row = dict(fixture=name, arguments=mode,
                   fixture_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest(),
                   oracle_stdout_sha256=hashlib.sha256(expected).hexdigest(),
                   returncode=actual.returncode,
                   passed=(actual.returncode, actual.stdout, actual.stderr) == (0, expected, b''))
        report['rows'].append(row)
        assert row['passed'], (stem, actual)
        print('PASS', stem, flush=True)
    report['passed'] = True
finally:
    (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
