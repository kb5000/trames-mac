#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
import json
from pathlib import Path
import runpy
import subprocess
root = Path(__file__).resolve().parents[1]
pin = json.loads((root / 'runtime-version.json').read_text())
actual = subprocess.check_output(['git', '-C', str(root / 'runtime'), 'rev-parse', 'HEAD'], text=True).strip()
assert actual == pin['revision'], ('runtime revision mismatch', actual, pin['revision'])
assert not subprocess.check_output(['git', '-C', str(root / 'runtime'), 'status', '--porcelain'], text=True).strip(), 'runtime has uncommitted changes'
assert subprocess.check_output(['git', 'config', '-f', str(root / '.gitmodules'), 'submodule.runtime.url'], text=True).strip() == pin['repository']
runpy.run_path(str(root / 'runtime/scripts/check-execution-boundaries.py'), run_name='__main__')
