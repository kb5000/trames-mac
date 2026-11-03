#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Real HVF Docker persistence/lifetime acceptance in a disposable registry.

The prepared, stopped Debian systemd store must contain docker.io and nginx:alpine.
Original store contents are copied by env import and are never modified.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import secrets
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--prepared-store', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    runner = str(args.runner.resolve())
    spec = importlib.util.spec_from_file_location('background_acceptance', Path(__file__).with_name('mac-background.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    records = []
    with tempfile.TemporaryDirectory(prefix='trames-docker-') as folder:
        home = Path(folder)
        env = dict(os.environ, TRAMES_HOME=str(home))

        def run(*command):
            p = subprocess.run([runner, *command], env=env, capture_output=True, text=True, timeout=120)
            records.append(dict(command=command, code=p.returncode, stdout=p.stdout, stderr=p.stderr))
            assert p.returncode == 0, records[-1]
            return p

        def terminal(name):
            return module.Terminal(runner, env, args.output, name, ['env', 'attach', 'docker'])

        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        token = 'persistent-' + secrets.token_hex(12)

        def http_once():
            with urllib.request.urlopen(f'http://127.0.0.1:{port}/', timeout=5) as response:
                assert response.read().decode() == token

        def http():
            deadline = time.monotonic() + 90
            while True:
                try:
                    http_once()
                    return
                except Exception:
                    assert time.monotonic() < deadline, 'container HTTP did not recover'
                    time.sleep(.2)

        def verify(name):
            http()
            t = terminal(name)
            try:
                t.command("stty -echo; timeout 60 sh -c 'until systemctl is-active --quiet docker; do sleep 0.2; done' && "
                          'test "$(docker inspect trames-acceptance --format "{{.State.Running}}")" = true && '
                          'test "$(docker exec trames-acceptance cat /usr/share/nginx/html/index.html)" = ' + token,
                          'PERSISTENCE', 90)
            finally:
                t.close()
            records.append(dict(check=name, passed=True))
            print('PASS', name, flush=True)

        run('env', 'import', 'docker', str(args.prepared_store.resolve()))
        run('env', 'configure', 'docker', '--init', 'systemd', '--cpus', '4', '--network', 'full',
            '--windows', 'off', '--vulkan', 'off', '--video', 'off', '--audio', 'off')
        agent = None
        try:
            run('env', 'start', 'docker', '--detach')
            t = terminal('setup')
            try:
                # These removals affect only the disposable imported copy.
                t.command("stty -echo; timeout 90 sh -c 'until systemctl is-active --quiet docker; do sleep 0.2; done' && "
                          'docker ps -aq | xargs -r docker rm -f', 'PREPARED', 120)
                t.command('docker volume create trames-acceptance && docker run -d --name trames-acceptance '
                          '--restart unless-stopped --log-opt max-size=1m --log-opt max-file=3 '
                          f'-p 127.0.0.1:{port}:80 -v trames-acceptance:/usr/share/nginx/html nginx:alpine && '
                          "docker exec trames-acceptance sh -c 'printf %s " + token + " > /usr/share/nginx/html/index.html && sync'",
                          'CONTAINER', 120)
                t.command('docker exec trames-acceptance nginx -t && docker exec trames-acceptance ps -o pid,ppid,args', 'DEFAULT_WORKERS')
                http()
                began = time.monotonic()
                with ThreadPoolExecutor(max_workers=8) as clients:
                    list(clients.map(lambda _: http_once(), range(64)))
                records.append(dict(check='concurrent_http', requests=64, clients=8, seconds=time.monotonic() - began, passed=True))
                t.command('systemctl restart docker', 'DOCKER_RESTART', 120)
            finally:
                t.close()
            verify('daemon_restart')
            run('env', 'stop', 'docker')
            run('env', 'start', 'docker', '--detach')
            verify('environment_restart')
            job = plistlib.loads((home / 'services/docker.plist').read_bytes())
            target = f'gui/{os.getuid()}/{job["Label"]}'

            def pid():
                p = subprocess.run(['/bin/launchctl', 'print', target], capture_output=True, text=True)
                m = re.search(r'\bpid = (\d+)', p.stdout)
                return int(m[1]) if m else None

            before = pid()
            assert before
            os.kill(before, signal.SIGKILL)
            deadline = time.monotonic() + 90
            while not pid() or pid() == before:
                assert time.monotonic() < deadline
                time.sleep(.2)
            verify('unsuccessful_host_exit')
            run('env', 'autostart', 'docker', 'on')
            agent = Path.home() / 'Library/LaunchAgents' / (job['Label'] + '.plist')
            assert plistlib.loads(agent.read_bytes())['ProgramArguments'] == job['ProgramArguments']
            run('env', 'stop', 'docker')
            subprocess.run(['/bin/launchctl', 'bootstrap', f'gui/{os.getuid()}', str(agent)], capture_output=True, check=True)
            verify('login_agent_bootstrap')
        finally:
            run('env', 'autostart', 'docker', 'off')
            run('env', 'stop', 'docker')
            if agent:
                assert not agent.exists()
            if (home / 'logs').exists():
                shutil.copytree(home / 'logs', args.output / 'service-logs', dirs_exist_ok=True)
            (args.output / 'results.json').write_text(json.dumps(dict(records=records), indent=2) + '\n')
    print('Default multiworker nginx, HTTP, volume persistence and all lifetime checks passed')


if __name__ == '__main__':
    main()
