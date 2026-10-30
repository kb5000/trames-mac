#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run installed Linux Firefox on native Vulkan/Zink with a separate profile.

Requires a signed native-driver Mac runner, an existing Firefox store, and
verified Linux graphics artifacts. No browser preferences or sandbox settings
are forced. See docs/validation/mac-firefox-gpu.md for build and validation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import uuid


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, default=root / 'target/release/trames-mac-run')
    parser.add_argument('--storage', type=Path, required=True, help='Existing Alpine store with Firefox and Mesa dependencies')
    parser.add_argument('--graphics-dir', type=Path, required=True, help='Fixed Mesa libEGL.so.1 and libgallium-25.1.9.so')
    parser.add_argument('--icd', type=Path, required=True, help='Linux libtrames_vulkan_icd.so')
    parser.add_argument('--loader', type=Path, required=True, help='Original Linux libvulkan.so.1')
    parser.add_argument('--video-driver', type=Path, help='Enable the paired VideoToolbox VA-API driver (H.264)')
    parser.add_argument('--profile', default='/root/firefox-native-gpu')
    parser.add_argument('--cpus', type=int, default=os.cpu_count() or 1)
    parser.add_argument('--timeout', type=int, default=86400)
    parser.add_argument('--network', choices=['off', 'loopback', 'outbound', 'full'], default='outbound')
    parser.add_argument('url', nargs='?', default='about:support')
    args = parser.parse_args()
    if args.cpus < 1 or args.timeout < 1 or not args.profile.startswith('/'):
        parser.error('positive CPU count/timeout and absolute guest profile path required')
    if not args.storage.is_dir():
        parser.error('--storage must be an existing Firefox store')
    files = {'libEGL.so.1': args.graphics_dir / 'libEGL.so.1',
             'libgallium-25.1.9.so': args.graphics_dir / 'libgallium-25.1.9.so',
             'libtrames_vulkan_icd.so': args.icd, 'libvulkan.so.1': args.loader}
    if args.video_driver:
        files['trames_drv_video.so'] = args.video_driver
    digest = hashlib.sha256()
    for name, path in sorted(files.items()):
        if not path.is_file():
            parser.error(f'missing artifact: {path}')
        digest.update(name.encode())
        with path.open('rb') as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(block)
    guest = '/opt/trames/graphics/' + digest.hexdigest()[:20]
    transfer = '/tmp/trames-gpu-import-' + uuid.uuid4().hex
    quote = shlex.quote
    with tempfile.TemporaryDirectory(prefix='trames-firefox-gpu-') as temporary:
        manifest = Path(temporary) / 'trames.json'
        manifest.write_text(json.dumps({'file_format_version': '1.0.0', 'ICD': {
            'api_version': '1.1.0', 'library_path': guest + '/libtrames_vulkan_icd.so'}}) + '\n')
        files['trames.json'] = manifest
        command = [str(args.runner.resolve()), '--storage', str(args.storage.resolve()),
                   '--vulkan', '--no-pty', '--cpus', str(args.cpus), '--memory-mib', 'auto',
                   '--network', args.network, '--timeout', str(args.timeout)]
        if args.video_driver:
            command += ['--video']
        setup = ['mkdir -p ' + quote(guest) + ' ' + quote(args.profile)]
        for name, path in files.items():
            incoming = transfer + '/' + name
            command += ['--exec-file', incoming + '=' + str(path.resolve())]
            setup.append('cp ' + quote(incoming) + ' ' + quote(guest + '/' + name))
            setup.append('rm ' + quote(incoming))
        if args.video_driver:
            setup.append('export LIBVA_DRIVER_NAME=trames LIBVA_DRIVERS_PATH=' + quote(guest) +
                         ' MOZ_DRM_DEVICE=/dev/dri/renderD128')
        setup += ['rmdir ' + quote(transfer),
                  'export LD_LIBRARY_PATH=' + quote(guest),
                  'export VK_DRIVER_FILES=' + quote(guest + '/trames.json'),
                  'export GALLIUM_DRIVER=zink MESA_LOADER_DRIVER_OVERRIDE=zink',
                  'export LIBGL_KOPPER_WAYLAND_NO_DRM=true MOZ_ENABLE_WAYLAND=1',
                  'probe=$(/usr/lib/firefox/glxtest --wayland)',
                  'printf "%s\\n" "$probe"',
                  'printf "%s\\n" "$probe" | grep -q "OpenGL ES 3\\."',
                  'printf "%s\\n" "$probe" | grep -q MOLTENVK',
                  'exec firefox --no-remote --profile ' + quote(args.profile) + ' ' + quote(args.url)]
        return subprocess.run(command + ['/bin/sh', '-ec', '; '.join(setup)]).returncode


if __name__ == '__main__':
    raise SystemExit(main())
