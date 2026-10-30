#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Validate native GLES MSAA and resolve paths on an isolated installed-Mesa store.

The store must be disposable, contain the graphics libraries' distro dependencies,
and have no concurrent runner. This does not run the full GLES conformance suite.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import shlex
import subprocess
import uuid

p = argparse.ArgumentParser(description=__doc__)
for name in ('runner', 'store', 'fixture', 'icd', 'loader', 'graphics-dir', 'gles', 'output'):
    p.add_argument('--' + name, type=Path, required=True)
p.add_argument('--cpus', type=int, nargs='+', default=[1, 12])
a = p.parse_args()
if any(n < 1 for n in a.cpus):
    p.error('positive CPU counts required')
files = {'libEGL.so.1': a.graphics_dir / 'libEGL.so.1',
         'libgallium-25.1.9.so': a.graphics_dir / 'libgallium-25.1.9.so',
         'libGLESv2.so.2': a.gles, 'libtrames_vulkan_icd.so': a.icd,
         'libvulkan.so.1': a.loader, 'zink-msaa': a.fixture}
for path in [a.runner, a.store / 'manifest', *files.values()]:
    if not path.is_file():
        p.error('missing input: ' + str(path))
a.output.mkdir(parents=True, exist_ok=False)
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
prefix = '/tmp/trames-zink-msaa-' + uuid.uuid4().hex
manifest = a.output / 'icd.json'
manifest.write_text(json.dumps({'file_format_version': '1.0.0', 'ICD': {
    'api_version': '1.2.0', 'library_path': prefix + '/libtrames_vulkan_icd.so'}}) + '\n')
files['icd.json'] = manifest
base = [str(a.runner.resolve()), '--storage', str(a.store.resolve()), '--network', 'off',
        '--memory-mib', 'auto', '--no-pty', '--no-windows', '--timeout', '90']
report = {'passed': False, 'scope': 'GLES multisample renderbuffers, masks, alpha coverage and resolve; not full CTS',
          'runner_sha256': sha(a.runner), 'inputs': {name: sha(path) for name, path in files.items()},
          'cases': []}
imports = [v for name, path in files.items() for v in ('--exec-file', prefix + '/' + name + '=' + str(path.resolve()))]
setup = ('export LD_LIBRARY_PATH=' + shlex.quote(prefix) +
         ' GALLIUM_DRIVER=zink MESA_LOADER_DRIVER_OVERRIDE=zink LIBGL_KOPPER_WAYLAND_NO_DRM=true VK_DRIVER_FILES=' +
         shlex.quote(prefix + '/icd.json') + '; exec ' + shlex.quote(prefix + '/zink-msaa'))
try:
    for cpus in a.cpus:
        command = [*base, '--cpus', str(cpus), '--vulkan', '--metrics', *imports, '/bin/sh', '-ec', setup]
        imports = []
        result = subprocess.run(command, capture_output=True, timeout=100)
        (a.output / f'{cpus}.stdout').write_bytes(result.stdout)
        (a.output / f'{cpus}.stderr').write_bytes(result.stderr)
        markers = (b'OpenGL ES 3.1 Mesa 25.1.9', b'MOLTENVK', b'zink-msaa-case-ok', b'zink-msaa-ok')
        teardown = re.search(rb'with (\d+) MB of GPU memory still allocated', result.stderr)
        passed = (result.returncode == 0 and all(marker in result.stdout for marker in markers)
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
