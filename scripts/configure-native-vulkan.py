#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Select the Mac native driver without silently changing linkage on rebuild.

An explicit verified static directory is remembered in the ignored target tree.
CPU/SDK CI builds may explicitly select external linkage; production packages
require the embedded driver. stdout contains only the selected directory.
"""
import hashlib
import json
import os
from pathlib import Path
import sys


def main():
    root = Path(__file__).resolve().parents[1]
    record = root / 'target/native-vulkan.json'
    mode = os.environ.get('TRAMES_VULKAN_LINKAGE', 'embedded')
    if mode == 'external':
        print('')
        return
    if mode != 'embedded':
        raise SystemExit('TRAMES_VULKAN_LINKAGE must be embedded or external')
    explicit = os.environ.get('TRAMES_MOLTENVK_STATIC_DIR')
    saved = json.loads(record.read_text()) if record.is_file() else None
    directory = explicit or (saved and saved.get('directory'))
    if not directory:
        raise SystemExit('Native Vulkan is required for the default Mac build. Set '
                         'TRAMES_MOLTENVK_STATIC_DIR to the verified macOS MoltenVK '
                         'archive directory once; subsequent builds remember it. '
                         'For an explicit SDK/CPU-only build use TRAMES_VULKAN_LINKAGE=external.')
    directory = Path(directory).resolve()
    library = directory / 'libMoltenVK.a'
    if not library.is_file():
        raise SystemExit(f'Missing configured native Vulkan archive: {library}')
    data = library.read_bytes()
    if b'_trames_mvk_observe_fence' not in data:
        raise SystemExit('The native archive lacks the required completion adapter; '
                         'use the pinned patched MoltenVK build recipe.')
    digest = hashlib.sha256(data).hexdigest()
    if not explicit and saved.get('sha256') != digest:
        raise SystemExit('Configured native Vulkan archive changed; select the verified '
                         'directory explicitly with TRAMES_MOLTENVK_STATIC_DIR.')
    configuration = {'version': 1, 'directory': str(directory), 'sha256': digest}
    if saved != configuration:
        record.parent.mkdir(parents=True, exist_ok=True)
        temporary = record.with_suffix('.json.tmp')
        temporary.write_text(json.dumps(configuration, indent=2) + '\n')
        temporary.replace(record)
    print(directory)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError) as error:
        sys.exit(f'Cannot read native Vulkan build configuration: {error}')
