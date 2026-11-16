#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable native-manager configuration acceptance with a running HVF guest."""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('runner', 'alpine', 'output', 'installer', 'app'):
        parser.add_argument('--' + flag, type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    runner = str(args.runner.resolve())
    records = []
    with tempfile.TemporaryDirectory(prefix='trames-manager-acceptance-') as folder:
        home = Path(folder)
        registry = home / 'registry'
        env = dict(os.environ, TRAMES_HOME=str(registry))

        def run(*command, body=None, ok=True):
            result = subprocess.run([runner, *command], env=env, input=body,
                                    capture_output=True, text=True, timeout=90)
            records.append(dict(command=command, code=result.returncode,
                                stdout=result.stdout, stderr=result.stderr))
            (args.output / 'commands.json').write_text(json.dumps(records, indent=2) + '\n')
            assert (result.returncode == 0) == ok, records[-1]
            return result.stdout if ok else result.stderr

        def snapshot(name='first'):
            value = json.loads(run('_manager', 'snapshot'))
            return next(row for row in value['environments'] if row['name'] == name)

        def patch(row, changes, operation='save', ok=True):
            return run('_manager', operation, row['config']['environment']['uuid'],
                       body=json.dumps(dict(version=1, revision=row['revision'], changes=changes)), ok=ok)

        run('config', '--global', 'set', 'defaults.resources.cpus', '2',
            'defaults.display.windows', 'false', 'defaults.components.vulkan', 'false',
            'defaults.components.thunking', 'false', 'defaults.network.mode', 'none')
        children = [subprocess.Popen([runner, 'env', 'create', name, '--rootfs',
                                     str(args.alpine.resolve())], env=env,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    for name in ('first', 'second')]
        for child in children:
            stdout, stderr = child.communicate(timeout=90)
            assert child.returncode == 0, (stdout, stderr)
        first = snapshot()
        assert len(json.loads(run('_manager', 'snapshot'))['environments']) == 2
        profile = home / 'clone.toml'
        profile.write_text(first['toml'].replace('[image]\n', '[image]\nsource = ' + json.dumps(str(args.alpine.resolve())) + '\n'))
        run('env', 'create', 'clone', '--config', str(profile), '--new-identity')
        assert snapshot('clone')['config']['environment']['uuid'] != first['config']['environment']['uuid']
        assert snapshot('clone')['grants']['host_exec']

        try:
            run('env', 'start', 'first', '--detach')
            # The service is an existing managed guest, independent of the GUI.
            assert 'GUEST_LIVE' in run('-e', 'first', 'exec', '--', '/bin/echo', 'GUEST_LIVE')
            running = snapshot()
            assert running['state'].startswith('running'), running
            grant = Path(running['path']) / 'grants.toml'
            grants_before = grant.read_bytes()
            result = json.loads(patch(running, {'resources.memory': '4GiB',
                                               'interop.host_exec.enabled': False}))
            assert {change['path'] for change in result['changes']} == {
                'resources.memory', 'interop.host_exec.enabled'}
            assert grant.read_bytes() == grants_before
            assert 'STILL_LIVE' in run('-e', 'first', 'exec', '--', '/bin/echo', 'STILL_LIVE')
            assert snapshot()['state'] == running['state']
            assert snapshot()['grants']['host_exec']
            assert 'changed outside' in patch(running, {'resources.memory': '8GiB'}, ok=False)
            current = snapshot()
            # Native forms must send real booleans, including switch selections.
            # Both states round-trip through the same strict TOML model.
            for enabled in (False, True):
                patch(current, {key: enabled for key in (
                    'display.windows', 'components.audio', 'components.audio_input',
                    'components.vulkan', 'components.vaapi')})
                current = snapshot()
                for key in ('audio', 'audio_input', 'vulkan', 'vaapi'):
                    assert current['config']['components'][key] is enabled
                assert current['config']['display']['windows'] is enabled
            for setting in ('off', 'on'):
                run('_manager', 'permission', current['config']['environment']['uuid'],
                    'clipboard', setting, current['revision'])
                current = snapshot()
                assert current['config']['interop']['clipboard']['enabled'] is (setting == 'on')
                assert current['grants']['clipboard'] is (setting == 'on')
            assert 'PERMISSIONS_LIVE' in run('-e', 'first', 'exec', '--', '/bin/echo', 'PERMISSIONS_LIVE')
            saved = Path(current['profile']).read_bytes()
            patch(current, {'resources.cpus': 0}, ok=False)
            assert Path(current['profile']).read_bytes() == saved
            run('network', 'create', 'private', '--internal')
            patch(current, {'network.mode': 'bridge', 'network.bridge': 'missing'}, ok=False)
            assert Path(current['profile']).read_bytes() == saved
            patch(current, {'network.mode': 'bridge', 'network.bridge': 'private',
                            'network.publish': [{'protocol': 'tcp', 'host': '127.0.0.1:39876', 'guest': 80}]})
            assert 'UNINTERRUPTED' in run('-e', 'first', 'exec', '--', '/bin/echo', 'UNINTERRUPTED')
        finally:
            run('env', 'stop', 'first')

        # Reinstall must retain detached-session launchers while updating their binary path.
        launchers = registry / 'launchers'
        launchers.mkdir(exist_ok=True)
        normal = launchers / 'first.command'
        session = launchers / 'first-42.command'
        prefix = '#!/bin/sh\nexport TRAMES_HOME=' + shlex.quote(str(registry)) + '\nexec '
        normal.write_text(prefix + shlex.quote(runner) + ' -e first shell\n')
        session.write_text(prefix + shlex.quote(runner) + ' -e first session attach 42\n')
        install_env = dict(env, TRAMES_APPLICATIONS_DIR=str(home / 'apps'),
                           TRAMES_BIN_DIR=str(home / 'bin'), TRAMES_MODIFY_PATH='0')
        installed = subprocess.run(['bash', str(args.installer.resolve()), str(args.app.resolve())],
                                   env=install_env, capture_output=True, text=True, timeout=90)
        assert installed.returncode == 0, (installed.stdout, installed.stderr)
        assert ' session attach 42\n' in session.read_text()
        assert str(home / 'apps/Trames Dev.app/Contents/MacOS/trames-mac-run') in session.read_text()
        assert normal.read_text().endswith(' -e first shell\n')
        run('env', 'terminal', 'first', '--session', '0', ok=False)
    (args.output / 'summary.json').write_text(json.dumps(dict(
        status='passed', simultaneous_creation=True, profile_fresh_identity=True,
        live_next_start_configuration=True, grant_separation=True, stale_revision=True,
        atomic_validation=True, bridge_validation=True, session_launcher_upgrade=True,
        boolean_roundtrip=True, combined_permissions=True), indent=2) + '\n')
    print('PASS: manager model, concurrent creation, live guest save, grants, conflicts and session launcher upgrade')


if __name__ == '__main__':
    main()
