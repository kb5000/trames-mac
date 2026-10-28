#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Real HVF feature instructions and reproducible working-set timing/correctness.

Timings are evidence, never a machine-dependent pass/fail threshold. Run without
other benchmarks/builds for comparisons and retain normal background-load notes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--runner', type=Path, required=True)
p.add_argument('--features', type=Path, required=True)
p.add_argument('--working-set', type=Path, required=True)
p.add_argument('--mib', type=int, default=512)
p.add_argument('--repeats', type=int, default=3)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
assert a.mib > 0 and a.repeats > 0
a.output.mkdir(parents=True, exist_ok=False)
report = dict(passed=False, binaries={name: hashlib.sha256(path.read_bytes()).hexdigest()
              for name, path in [('runner', a.runner), ('features', a.features),
                                 ('working-set', a.working_set)]}, rows=[])
try:
    for cpus in (1, 4, 12):
        for kind, fixture, arguments, repeats in (
                ('features', a.features, [], 1),
                ('working-set', a.working_set, [str(cpus), str(a.mib)], a.repeats)):
            for repeat in range(repeats):
                command = [str(a.runner.resolve()), '--cpus', str(cpus),
                           '--memory-mib', '4096', '--network', 'off', '--no-pty',
                           '--no-windows', '--timeout', '60', str(fixture.resolve()), *arguments]
                result = subprocess.run(command, capture_output=True, text=True, timeout=70)
                name = f'{kind}-{cpus}-{repeat}'
                (a.output / (name + '.stdout')).write_text(result.stdout)
                (a.output / (name + '.stderr')).write_text(result.stderr)
                assert result.returncode == 0, (name, result.returncode, result.stderr)
                if kind == 'features':
                    assert 'advertised instruction subset ok' in result.stdout, result.stdout
                    detail = dict(output=result.stdout.splitlines())
                else:
                    detail = json.loads(result.stdout)
                    expected = sum((offset ^ 0x513a77) * 8 for offset in range(0, a.mib << 20, 4096))
                    assert detail['checksum'] == expected, detail
                report['rows'].append(dict(kind=kind, cpus=cpus, repeat=repeat, **detail))
                print('PASS', name, flush=True)
    report['passed'] = True
finally:
    (a.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
