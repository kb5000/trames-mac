#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run unchanged Linux Vulkan fixtures through HVF without a native window.

Inputs are previously verified AArch64 fixtures/ICD/loader and an official or
source-built Mac MoltenVK library. Logs and generated payloads belong in ignored
local output, not published browser/host traces. No network or UI input is used.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import time

parser = argparse.ArgumentParser(description=__doc__)
for flag in ('runner', 'rootfs', 'fixtures', 'loader', 'icd', 'library', 'output'):
    parser.add_argument('--' + flag, type=Path, required=True)
parser.add_argument('--static-driver', type=Path, help='archive used to build a statically linked runner, for provenance')
parser.add_argument('--extended', action='store_true', help='also run timeline waits with one/four CPUs and resource lifetime fixtures')
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
files = {'usr/lib/libvulkan.so.1': args.loader,
         'usr/lib/libtrames_vulkan_icd.so': args.icd}
names = ['vulkan-compute', 'vulkan-compute.spv', 'vulkan-render',
         'vulkan-render.vert.spv', 'vulkan-render.frag.spv']
if args.extended:
    names += ['vulkan-timeline', 'vulkan-resources', 'vulkan-resources.spv']
for name in names:
    files['tmp/' + name] = args.fixtures / name
payload = args.output / 'payload.tar'
with tarfile.open(payload, 'w') as archive:
    for name, path in files.items():
        data = path.read_bytes()
        item = tarfile.TarInfo(name)
        item.size, item.mode = len(data), 0o755
        archive.addfile(item, io.BytesIO(data))
    data = json.dumps({'file_format_version': '1.0.0', 'ICD': {
        'api_version': '1.1.0', 'library_path': '/usr/lib/libtrames_vulkan_icd.so'}}).encode()
    item = tarfile.TarInfo('usr/share/vulkan/icd.d/trames.json')
    item.size, item.mode = len(data), 0o644
    archive.addfile(item, io.BytesIO(data))
cases = [('disabled', 4, False, 'disabled'), ('compute-one', 1, True, ''),
         ('compute-four', 4, True, ''), ('coherent', 4, True, 'coherent'),
         ('template', 4, True, 'template'), ('render-headless', 4, True, None)]
if args.extended:
    cases += [('timeline-one', 1, True, 'timeline'), ('timeline-four', 4, True, 'timeline'),
              ('resources', 4, True, 'resources')]
rows = []
report = {'passed': False, 'rows': rows,
          'inputs': {name: hashlib.sha256(path.read_bytes()).hexdigest()
                     for name, path in files.items()},
          'runner_sha256': hashlib.sha256(args.runner.read_bytes()).hexdigest(),
          'library_argument_sha256': hashlib.sha256(args.library.read_bytes()).hexdigest(),
          'driver_binding': 'static' if args.static_driver else 'dynamic override or embedded; not inferred',
          'static_driver_sha256': hashlib.sha256(args.static_driver.read_bytes()).hexdigest() if args.static_driver else None,
          'rootfs_sha256': hashlib.sha256(args.rootfs.read_bytes()).hexdigest()}
try:
    for name, cpus, enabled, mode in cases:
        guest = ('/tmp/vulkan-render /tmp/vulkan-render.vert.spv '
                 '/tmp/vulkan-render.frag.spv /tmp/render.ppm' if mode is None else
                 '/tmp/vulkan-compute /tmp/vulkan-compute.spv ' + mode)
        if mode == 'timeline':
            guest = '/tmp/vulkan-timeline'
        elif mode == 'resources':
            guest = '/tmp/vulkan-resources /tmp/vulkan-resources.spv'
        command = [str(args.runner.resolve()), '--rootfs', str(args.rootfs.resolve()),
                   '--network', 'off', '--cpus', str(cpus), '--no-pty', '--no-windows',
                   '--metrics', '--timeout', '90', '--exec-file',
                   '/tmp/device.tar=' + str(payload.resolve())]
        if enabled:
            command.append('--vulkan')
        command += ['/bin/sh', '-ec', 'tar -xf /tmp/device.tar -C /; '
                    'export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/trames.json; exec ' + guest]
        # The disabled control must not initialize the native driver.
        env = dict(os.environ, TRAMES_MAC_WINDOW_TRACE='1',
                   TRAMES_VULKAN_LIBRARY=str(args.library.resolve()) if enabled else '/nonexistent/trames-disabled-driver')
        began = time.monotonic()
        result = subprocess.run(command, capture_output=True, env=env, timeout=100)
        (args.output / (name + '.stdout')).write_bytes(result.stdout)
        (args.output / (name + '.stderr')).write_bytes(result.stderr)
        marker = (b'vulkan-render-ok' if mode is None else
                  b'vulkan-compute-ok' if enabled else b'vulkan-disabled-ok')
        if mode in ('timeline', 'resources'):
            marker = ('vulkan-' + mode + '-ok').encode()
        assert result.returncode == 0 and marker in result.stdout, (name, result.returncode, result.stdout, result.stderr)
        assert b'trames mac windows:' not in result.stderr, (name, result.stderr)
        metrics = re.search(rb'trames mac native: calls=(\d+).*upload_bytes=(\d+) download_bytes=(\d+) direct_bytes=(\d+) buffered_bytes=(\d+)', result.stderr)
        assert metrics, (name, result.stderr)
        calls, upload, download, direct, buffered = map(int, metrics.groups())
        assert (calls > 0) == enabled, (name, calls)
        if enabled:
            assert b'with 0 MB of GPU memory still allocated' in result.stderr, (name, result.stderr)
        else:
            assert b'[mvk-' not in result.stderr, result.stderr
        rows.append({'name': name, 'cpus': cpus, 'passed': True,
                     'seconds': round(time.monotonic() - began, 4), 'calls': calls,
                     'upload_bytes': upload, 'download_bytes': download,
                     'direct_fragment_bytes': direct, 'buffered_fragment_bytes': buffered})
        shared = re.search(rb'trames mac shared GPU: maps=(\d+) mapped_bytes=(\d+)', result.stderr)
        coherent = re.search(rb'trames mac coherent: boundaries=(\d+) ranges=(\d+) upload_scanned_bytes=(\d+) download_scanned_bytes=(\d+)', result.stderr)
        if shared:
            rows[-1].update(shared_maps=int(shared[1]), shared_mapped_bytes=int(shared[2]))
        if coherent:
            rows[-1].update(coherent_boundaries=int(coherent[1]), coherent_ranges=int(coherent[2]),
                            coherent_upload_scanned_bytes=int(coherent[3]), coherent_download_scanned_bytes=int(coherent[4]))
        print(json.dumps(rows[-1]), flush=True)
    report['passed'] = True
finally:
    (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
