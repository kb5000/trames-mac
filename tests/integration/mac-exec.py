#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable real-HVF Unix CLI, structured exec and independent shell acceptance."""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import runpy
import signal
import subprocess
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner', type=Path, required=True)
    p.add_argument('--alpine', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    runner = str(a.runner.resolve())
    Terminal = runpy.run_path(str(Path(__file__).with_name('mac-background.py')))['Terminal']
    checks = []
    with tempfile.TemporaryDirectory(prefix='trames-exec-') as temp:
        home = Path(temp)
        env = dict(os.environ, TRAMES_HOME=str(home / 'registry'))
        clients = []

        def run(*args, data=b'', code=0, timeout=30):
            result = subprocess.run([runner, *args], input=data, env=env,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
            assert result.returncode == code, (args, result.returncode, result.stdout[:200], result.stderr[:1000])
            return result

        def execute(*args, **kwargs):
            return run('-e', 'lab', 'exec', *args, **kwargs)

        try:
            assert b'Usage: trames' in run().stdout
            assert b'Usage: trames' in run('exec', '--help').stdout
            for shell in ('bash', 'zsh', 'fish'):
                assert run('completion', shell).stdout
            completion = run('completion', 'zsh').stdout
            subprocess.run(['/bin/zsh', '-f', '-c', 'autoload -Uz compinit; compinit -D; source /dev/stdin; [[ $_comps[trames] == _trames ]]'], input=completion, check=True)
            assert not (home / 'registry').exists(), 'help/completion must not initialize a registry'
            run('-e', 'missing', 'exec', '--', 'true', code=1)
            assert not (home / 'registry/environments/missing').exists()
            assert run('config', '--global', 'get', 'defaults.components.vulkan').stdout == b'true\n'
            run('config', '--global', 'set', 'defaults.image.distribution', 'debian')
            run('config','--global','set','defaults.image.distribution','alpine')
            assert run('config', '--global', 'get', 'defaults.image.distribution').stdout == b'alpine\n'
            run('env', 'create', 'lab', '--archive', str(a.alpine.resolve()), '--vulkan','false','--network','none')
            run('config', '--global', 'set', 'default_environment', 'lab')
            run('config', 'set','resources.cpus','all','resources.memory','4GiB')
            assert run('config', 'get','resources.memory').stdout == b'4GiB\n'
            run('config','set','resources.memory','auto')
            profile = Path(run('-e','lab','config','path').stdout.decode().strip())
            before = profile.read_bytes()
            run('config', 'set','resources.cpus','0', code=1)
            assert profile.read_bytes() == before
            assert run('env', 'ls', '-q').stdout == b'lab\n'
            assert run('env', 'ls').stdout.startswith(b'NAME')
            rows = json.loads(run('env', 'list', '--json').stdout)
            assert not rows[0]['config']['components']['vulkan'] and rows[0]['config']['resources']['memory'] == 'auto'
            for _ in range(2):
                assert run('env', 'start', 'lab').stdout == b''
            assert execute('--', 'sh', '-c', 'echo out; echo err >&2; exit 7', code=7).stdout == b'out\n'
            assert execute('--', 'sh', '-c', 'echo err >&2').stderr == b'err\n'
            assert execute('--', 'printf', '%s', 'a b;$(literal)').stdout == b'a b;$(literal)'
            assert execute('--', 'printf', '%s', '-e').stdout == b'-e'
            assert execute('--setenv', 'X=a b', '--cwd', '/tmp', '--', 'sh', '-c', 'printf "%s:%s" "$X" "$PWD"').stdout == b'a b:/tmp'
            assert execute('-t', '--', 'sh', '-c', 'test -t 0 && test -t 1 && printf tty').stdout == b'tty'
            execute('--', 'command-that-does-not-exist', code=127)
            execute('--', '/etc/os-release', code=126)
            assert run('-e', 'lab', '--', 'printf', '%s', 'short').stdout == b'short'
            payload = bytes(range(256)) * 32768
            assert execute('--', 'cat', data=payload).stdout == payload
            streams = execute('--', 'sh', '-c', 'head -c 1048576 /dev/zero; head -c 1048576 /dev/zero >&2')
            assert streams.stdout == streams.stderr == bytes(1 << 20)
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                futures = [pool.submit(execute, '--', 'sh', '-c', f'sleep 0.05; printf job{i}') for i in range(4)]
                assert [f.result().stdout for f in futures] == [f'job{i}'.encode() for i in range(4)]
            child = subprocess.Popen([runner, '-e', 'lab', 'exec', '--', 'sh', '-c', 'echo READY; sleep 30'],
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
            try:
                assert child.stdout.readline() == b'READY\n'
                child.send_signal(signal.SIGINT)
                out, err = child.communicate(timeout=10)
                assert child.returncode == 130 and not out and not err
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait()
            for name in ('a', 'b'):
                clients.append(Terminal(runner, env, a.output, name, ['-e', 'lab', 'shell']))
            sessions = json.loads(run('-e', 'lab', 'session', 'ls', '--json').stdout)
            assert {c.id for c in clients}.issubset({s['id'] for s in sessions})
            assert all(s['attached'] for s in sessions)
            clients[0].command('echo FIRST > /tmp/shared', 'SHELL_A')
            assert execute('--', 'cat', '/tmp/shared').stdout == b'FIRST\n'
            clients[1].command('true', 'SHELL_B')
            for c in clients:
                c.close()
            clients.clear()
            assert execute('--', 'echo', 'alive').stdout == b'alive\n'
            run('env', 'enable', 'lab')
            assert json.loads(run('env', 'status', 'lab').stdout)['autostart']
            run('env', 'disable', 'lab')
            assert json.loads(run('env', 'status', 'lab').stdout)['running']
            run('env', 'restart', 'lab')
            assert execute('--', 'cat', '/tmp/shared').stdout == b'FIRST\n'
            run('env', 'enable', 'lab', '--now')
            run('env', 'disable', 'lab', '--now')
            assert not json.loads(run('env', 'status', 'lab').stdout)['running']
            checks.extend(['help_without_runtime', 'no_implicit_creation', 'scoped_config_and_defaults', 'idempotent_background_start',
                           'literal_argv_env_cwd', 'stdout_stderr_exit', 'binary_full_duplex_8MiB', 'pty_opt_in',
                           'concurrent_exec', 'signal_group_isolation', 'independent_shells_and_listing', 'persistent_restart', 'login_enable_disable'])
        finally:
            for c in clients:
                c.close()
            subprocess.run([runner, 'env', 'disable', 'lab', '--now'], env=env, capture_output=True, timeout=90)
    (a.output / 'results.json').write_text(json.dumps(dict(passed=True, checks=checks,
        runner_sha256=hashlib.sha256(Path(runner).read_bytes()).hexdigest()), indent=2) + '\n')
    print('PASS: Unix CLI, structured exec, binary streams, signals, independent shells and lifecycle')


if __name__ == '__main__':
    main()
