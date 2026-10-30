#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Validate promoted Vulkan 1.2 core shader outputs in an isolated store."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import uuid

p = argparse.ArgumentParser(description=__doc__)
for name in ('runner', 'store', 'inputs', 'loader', 'output'):
    p.add_argument('--' + name, type=Path, required=True)
p.add_argument('--cpus', type=int, nargs='+', default=[1, 12])
a = p.parse_args()
if any(n < 1 for n in a.cpus):
    p.error('positive CPU counts required')
files = {name: a.inputs / name for name in ('libtrames_vulkan_icd.so', 'vulkan-core12', 'vulkan-core12.spv')}
files['libvulkan.so.1'] = a.loader
for path in [a.runner, a.store / 'manifest', *files.values()]:
    if not path.is_file():
        p.error('missing input: ' + str(path))
a.output.mkdir(parents=True, exist_ok=False)
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
prefix = '/tmp/trames-core12-' + uuid.uuid4().hex
manifest = a.output / 'icd.json'
manifest.write_text(json.dumps({'file_format_version': '1.0.0', 'ICD': {
    'api_version': '1.2.0', 'library_path': prefix + '/libtrames_vulkan_icd.so'}}) + '\n')
files['icd.json'] = manifest
base = [str(a.runner.resolve()), '--storage', str(a.store.resolve()), '--network', 'off',
        '--memory-mib', 'auto', '--no-pty', '--no-windows', '--timeout', '90']
report = {'passed': False, 'scope': 'selected Vulkan 1.2 core/SPIR-V 1.5 outputs and large mapping; not complete CTS',
          'runner_sha256': sha(a.runner), 'inputs': {name: sha(path) for name, path in files.items()}, 'cases': []}
imports = [v for name, path in files.items() for v in ('--exec-file', prefix + '/' + name + '=' + str(path.resolve()))]
setup = ('export LD_LIBRARY_PATH=' + shlex.quote(prefix) + ' VK_DRIVER_FILES=' + shlex.quote(prefix + '/icd.json') +
         '; exec ' + shlex.quote(prefix + '/vulkan-core12') + ' ' + shlex.quote(prefix + '/vulkan-core12.spv'))
try:
    for cpus in a.cpus:
        result = subprocess.run([*base, '--cpus', str(cpus), '--vulkan', '--metrics', *imports,
                                 '/bin/sh', '-ec', setup], capture_output=True, timeout=100)
        imports = []
        (a.output / f'{cpus}.stdout').write_bytes(result.stdout)
        (a.output / f'{cpus}.stderr').write_bytes(result.stderr)
        passed = (result.returncode == 0 and all(marker in result.stdout for marker in
                  (b'core12 device=', b'core12-gpu-ok:', b'core12-samplers-ok:', b'core12-ok'))
                  and b'with 0 MB of GPU memory still allocated' in result.stderr
                  and b'trames mac windows:' not in result.stderr)
        report['cases'].append({'cpus': cpus, 'exit_code': result.returncode, 'passed': passed})
        print(cpus, 'passed' if passed else 'failed', flush=True)
        if not passed:
            break
    report['passed'] = len(report['cases']) == len(a.cpus) and all(row['passed'] for row in report['cases'])
finally:
    cleanup = subprocess.run([*base, '--cpus', '1', '/bin/sh', '-ec',
                              'rm -f ' + ' '.join(shlex.quote(prefix + '/' + name) for name in files) +
                              '; rmdir ' + shlex.quote(prefix)], capture_output=True, timeout=30)
    report['cleanup_exit'] = cleanup.returncode
    (a.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
raise SystemExit(0 if report['passed'] and report['cleanup_exit'] == 0 else 1)
