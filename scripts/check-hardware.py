#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build, ad-hoc sign, and run explicit Apple Silicon hardware acceptance tests."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=Path('target/hardware-tests/results.json'))
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
assert platform.system() == 'Darwin' and platform.machine() == 'arm64', 'Apple Silicon macOS required'
build = subprocess.run(['cargo', 'test', '--workspace', '--no-run', '--message-format=json'],
                       cwd=root, stdout=subprocess.PIPE, text=True, check=True)
executables = {}
for line in build.stdout.splitlines():
    record = json.loads(line)
    if record.get('reason') == 'compiler-artifact' and record.get('executable') and record['profile']['test']:
        if record['target']['name'] in ('trames_hvf', 'trames-mac-run'):
            executables[record['target']['name']] = Path(record['executable'])
assert len(executables) == 2, executables
report = dict(passed=False, runtime_revision=json.loads((root / 'runtime-version.json').read_text())['revision'], rows=[])
args.output.parent.mkdir(parents=True, exist_ok=True)
try:
    for name, executable in sorted(executables.items()):
        subprocess.run(['codesign', '--force', '--sign', '-', '--entitlements',
                        str(root / 'apps/macos/Trames.entitlements'), str(executable)], check=True)
        result = subprocess.run([str(executable), '--ignored', '--test-threads=1', '--nocapture'],
                                cwd=root, capture_output=True, text=True, timeout=60)
        print(result.stdout + result.stderr, end='', flush=True)
        (args.output.parent / (name + '.log')).write_text(result.stdout + result.stderr)
        count = re.search(r'test result: ok\. (\d+) passed;', result.stdout)
        row = dict(target=name, executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                   exit=result.returncode, passed_tests=int(count.group(1)) if count else 0)
        report['rows'].append(row)
        assert result.returncode == 0 and row['passed_tests'] > 0, row
    report['passed'] = True
finally:
    args.output.write_text(json.dumps(report, indent=2) + '\n')
