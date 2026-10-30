#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Validate promoted Vulkan 1.1/1.2 GPU outputs in an isolated store."""
import argparse
import hashlib
import json
import re
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
files = {name: a.inputs / name for name in ('libtrames_vulkan_icd.so', 'vulkan12-complete', 'vulkan12-index.comp.spv', 'vulkan12-views.vert.spv', 'vulkan12-views.frag.spv', 'vulkan12-input.frag.spv', 'vulkan12-ycbcr.comp.spv', 'vulkan12-atomic64.comp.spv', 'vulkan12-multiview.frag.spv')}
files['libvulkan.so.1'] = a.loader
for path in [a.runner, a.store / 'manifest', *files.values()]:
    if not path.is_file():
        p.error('missing input: ' + str(path))
a.output.mkdir(parents=True, exist_ok=False)
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
prefix = '/tmp/trames-vulkan12-' + uuid.uuid4().hex
manifest = a.output / 'icd.json'
manifest.write_text(json.dumps({'file_format_version': '1.0.0', 'ICD': {
    'api_version': '1.2.0', 'library_path': prefix + '/libtrames_vulkan_icd.so'}}) + '\n')
files['icd.json'] = manifest
base = [str(a.runner.resolve()), '--storage', str(a.store.resolve()), '--network', 'off',
        '--memory-mib', 'auto', '--no-pty', '--no-windows', '--timeout', '120']
report = {'passed': False, 'scope': 'native multiview, descriptor indexing, depth/stencil resolve, subpasses, secondary commands, events, linear images and YCbCr output oracles; not complete CTS',
          'runner_sha256': sha(a.runner), 'inputs': {name: sha(path) for name, path in files.items()}, 'teardown_metric_scope': 'Metal currentAllocatedSize before MTLDevice release; includes native driver/autoreleased resources, not a Vulkan live-object count', 'cases': []}
imports = [v for name, path in files.items() for v in ('--exec-file', prefix + '/' + name + '=' + str(path.resolve()))]
setup = ('export LD_LIBRARY_PATH=' + shlex.quote(prefix) + ' VK_DRIVER_FILES=' + shlex.quote(prefix + '/icd.json') +
         '; exec ' + shlex.quote(prefix + '/vulkan12-complete') + ' ' + shlex.quote(prefix))
try:
    for cpus in a.cpus:
        result = subprocess.run([*base, '--cpus', str(cpus), '--vulkan', '--metrics', *imports,
                                 '/bin/sh', '-ec', setup], capture_output=True, timeout=130)
        imports = []
        (a.output / f'{cpus}.stdout').write_bytes(result.stdout)
        (a.output / f'{cpus}.stderr').write_bytes(result.stderr)
        teardown = re.search(rb'with (\d+) MB of GPU memory still allocated', result.stderr)
        passed = (result.returncode == 0 and all(marker in result.stdout for marker in
                  (b'vulkan12 device=', b'vulkan12-descriptors-ok', b'vulkan12-resolve-ok', b'vulkan12-multiview-ok', b'vulkan12-subpasses-ok', b'vulkan12-events-ok', b'vulkan12-linear-ok', b'vulkan12-ycbcr-ok', b'vulkan12-allocation-ok', b'vulkan12-stencil-usage-ok', b'vulkan12-complete-ok'))
                  and teardown is not None
                  and b'trames mac windows:' not in result.stderr)
        report['cases'].append({'cpus': cpus, 'exit_code': result.returncode, 'passed': passed, 'metal_current_allocated_mib_at_physical_teardown': int(teardown[1]) if teardown else None})
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
