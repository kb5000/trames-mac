#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check stable/dev identity, parallel execution and non-overwriting installation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import time


def inventory(app):
    return {str(p.relative_to(app)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in app.rglob('*') if p.is_file()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--stable', type=Path, required=True)
    p.add_argument('--dev', type=Path, required=True)
    p.add_argument('--installer', type=Path, required=True)
    p.add_argument('--alpine', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    sources = dict(stable=a.stable.resolve(), dev=a.dev.resolve())
    rows = []
    for channel, app in sources.items():
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        assert info['TramesChannel'] == channel
        assert info['CFBundleIdentifier'] == 'org.trames.Trames.mac' + ('.dev' if channel == 'dev' else '')
        env = dict(os.environ, TRAMES_BUILD_CHANNEL='dev' if channel == 'stable' else 'stable')
        env.pop('TRAMES_HOME', None)
        path = subprocess.check_output([str(app / 'Contents/MacOS/trames-mac-run'), 'config', '--global', 'path'], env=env, text=True).strip()
        expected = 'Trames Dev' if channel == 'dev' else 'Trames'
        assert Path(path).parts[-2:] == (expected, 'cli.json'), path
        rows.append(dict(channel=channel, bundle_id=info['CFBundleIdentifier'], default_config=path))
    with tempfile.TemporaryDirectory(prefix='trames-channels-') as temp:
        base = Path(temp)
        env = dict(os.environ, SHELL='/bin/zsh', XDG_CONFIG_HOME=str(base / 'config'),
                   ZDOTDIR=str(base / 'profiles'), TRAMES_APPLICATIONS_DIR=str(base / 'Applications'),
                   TRAMES_BIN_DIR=str(base / 'bin'))
        installed = {}
        homes = {}
        for channel in ('stable', 'dev'):
            subprocess.run(['/bin/bash', str(a.installer.resolve()), str(sources[channel])], env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=30)
            installed[channel] = base / 'bin' / ('trames' if channel == 'stable' else 'trames-dev')
            homes[channel] = dict(env, TRAMES_HOME=str(base / (channel + '-registry')))
        stable_app = base / 'Applications/Trames.app'
        stable_before = inventory(stable_app)
        processes = {}
        handles = []
        try:
            for channel in ('stable', 'dev'):
                exe, scoped = str(installed[channel]), homes[channel]
                subprocess.run([exe, 'env', 'create', 'same-name', '--archive', str(a.alpine.resolve()),
                                '--cpus', '1', '--windows', 'off'], env=scoped, check=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
                out_path = base / (channel + '.stdout')
                out, err = out_path.open('w'), (base / (channel + '.stderr')).open('w')
                handles.extend([out, err])
                child = subprocess.Popen([exe, 'debug', 'env-run', 'same-name', '--no-pty', '/bin/sh', '-ec',
                                          f'echo {channel} > /root/channel; sync; echo CHANNEL_READY; sleep 300'],
                                         env=scoped, stdout=out, stderr=err)
                processes[channel] = child
                deadline = time.monotonic() + 20
                while 'CHANNEL_READY' not in out_path.read_text():
                    assert child.poll() is None and time.monotonic() < deadline, out_path
                    time.sleep(.05)
            # Reinstall the dev app while both environments are running. Neither
            # the stable bundle nor its process/store may be replaced or stopped.
            subprocess.run(['/bin/bash', str(a.installer.resolve()), str(sources['dev'])], env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=30)
            assert stable_before == inventory(stable_app)
            for channel in ('dev', 'stable'):
                subprocess.run([str(installed[channel]), 'env', 'stop', 'same-name'], env=homes[channel],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=20)
                processes[channel].wait(timeout=10)
                assert processes[channel].returncode == 0
                if channel == 'dev':
                    assert processes['stable'].poll() is None
                value = subprocess.check_output([str(installed[channel]), 'debug', 'env-run', 'same-name', '--no-pty',
                                                 '/bin/cat', '/root/channel'], env=homes[channel], text=True)
                assert value.strip() == channel
            query = subprocess.check_output(['/bin/zsh', '-lic', 'command -v trames; command -v trames-dev'], env=env, text=True)
            assert str(installed['stable']) in query and str(installed['dev']) in query
            text = (base / 'profiles/.zshrc').read_text()
            assert text.count('# Trames CLI') == 1 and text.count('# Trames Dev CLI') == 1
        finally:
            for child in processes.values():
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
            for handle in handles:
                handle.close()
    (a.output / 'result.json').write_text(json.dumps(dict(passed=True, channels=rows,
        parallel_execution=True, stable_unchanged_by_dev_install=True), indent=2) + '\n')
    print('PASS: stable/dev registry and bundle identity, parallel execution, independent stop, dev reinstall and CLI PATH')


if __name__ == '__main__':
    main()
