#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""A plain rebuild retains native linkage; stale config never becomes external."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix='trames-native-driver-') as temporary:
    work = Path(temporary)
    (work / 'scripts').mkdir()
    selector = work / 'scripts/configure-native-vulkan.py'
    shutil.copyfile(root / 'scripts/configure-native-vulkan.py', selector)
    library = work / 'driver/libMoltenVK.a'
    library.parent.mkdir()
    library.write_bytes(b'!<arch>\n_trames_mvk_observe_fence fixture')
    env = {k:v for k,v in os.environ.items()
           if k not in ('TRAMES_MOLTENVK_STATIC_DIR', 'TRAMES_VULKAN_LINKAGE')}
    def select(**values):
        return subprocess.run([sys.executable, str(selector)], env=dict(env, **values),
                              capture_output=True, text=True)
    assert select().returncode != 0
    explicit = select(TRAMES_MOLTENVK_STATIC_DIR=str(library.parent))
    assert explicit.returncode == 0 and explicit.stdout.strip() == str(library.parent.resolve())
    assert select().stdout == explicit.stdout
    record = (work / 'target/native-vulkan.json').read_bytes()
    external = select(TRAMES_VULKAN_LINKAGE='external')
    assert external.returncode == 0 and not external.stdout.strip()
    assert (work / 'target/native-vulkan.json').read_bytes() == record
    library.write_bytes(library.read_bytes() + b'changed')
    assert select().returncode != 0
    assert select(TRAMES_MOLTENVK_STATIC_DIR=str(library.parent)).returncode == 0
    assert select().returncode == 0
    library.unlink()
    assert select().returncode != 0
    assert select(TRAMES_VULKAN_LINKAGE='typo').returncode != 0
print('native Vulkan selection, persistence and stale/missing archive rejection passed')
