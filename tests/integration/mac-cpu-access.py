#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Linux EL0 timer access and recoverable privileged-register faults on HVF."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--runner', type=Path, required=True)
p.add_argument('--fixture', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
report = dict(passed=False, runner_sha256=hashlib.sha256(a.runner.read_bytes()).hexdigest(),
              fixture_sha256=hashlib.sha256(a.fixture.read_bytes()).hexdigest(), rows=[])
try:
    for cpus in (1, 4):
        for mode, expected in (
                ('counter', b'EL0 virtual counter, frequency and threaded monotonic time: ok\n'),
                ('privileged', b'privileged timer and EL1 access deliver recoverable SIGILL: ok\n')):
            command = [str(a.runner.resolve()), '--cpus', str(cpus), '--no-pty',
                       '--timeout', '10', '--metrics', str(a.fixture.resolve()), mode]
            result = subprocess.run(command, capture_output=True, timeout=15)
            stem = str(cpus) + '-' + mode
            (a.output / (stem + '.stdout')).write_bytes(result.stdout)
            (a.output / (stem + '.stderr')).write_bytes(result.stderr)
            passed = result.returncode == 0 and result.stdout == expected
            report['rows'].append(dict(cpus=cpus, mode=mode, command=command,
                                       exit=result.returncode, passed=passed))
            assert passed, (stem, result)
            print('PASS', stem, flush=True)
    report['passed'] = True
finally:
    (a.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
