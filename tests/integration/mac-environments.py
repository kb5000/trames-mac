#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable real-HVF product acceptance; no existing user stores are touched."""
import argparse
import gzip
import hashlib
import io
import json
import os
import pty
import termios
from pathlib import Path
import signal
import subprocess
import tarfile
import tempfile
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner', type=Path, required=True)
    p.add_argument('--alpine', type=Path, required=True)
    p.add_argument('--debian', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--soak-seconds', type=int, default=0)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    rows = []
    with tempfile.TemporaryDirectory(prefix='trames-env-') as tmp:
        home = Path(tmp)
        env = dict(os.environ, TRAMES_HOME=str(home))
        runner = str(a.runner.resolve())

        def run(*args, ok=True, timeout=180):
            began = time.monotonic()
            r = subprocess.run([runner, *args], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
            rows.append({'arguments': list(args), 'code': r.returncode, 'seconds': time.monotonic() - began, 'stdout': r.stdout, 'stderr': r.stderr})
            assert (r.returncode == 0) == ok, rows[-1]
            return r

        def start(name, command):
            return run('env', 'start', name, '--no-windows', '--no-pty', '/bin/sh', '-ec', command)

        def spawn(name, command):
            out = (home / 'live.stdout').open('w')
            err = (home / 'live.stderr').open('w')
            child = subprocess.Popen([runner, 'env', 'start', name, '--no-windows', '--no-pty', '/bin/sh', '-ec', command], env=env, stdout=out, stderr=err, text=True)
            deadline = time.monotonic() + 20
            while 'READY' not in (home / 'live.stdout').read_text():
                if child.poll() is not None or time.monotonic() > deadline:
                    raise AssertionError((child.poll(), (home / 'live.stderr').read_text()))
                time.sleep(.05)
            out.close()
            err.close()
            return child

        for distro, archive in [('alpine', a.alpine), ('debian', a.debian)]:
            run('env', 'create', distro, '--distro', distro, '--archive', str(archive.resolve()))
            run('env', 'configure', distro, '--cpus', '2', '--windows', 'off')
            start(distro, 'echo persistent > /root/check; sync; uname -m; cat /etc/resolv.conf; cat /etc/os-release')
            assert '169.254.0.53' in rows[-1]['stdout']
            assert 'persistent' in start(distro, 'cat /root/check').stdout
        assert 'glibc' in start('debian', 'getconf GNU_LIBC_VERSION').stdout
        run('env', 'create', 'alpine', '--archive', str(a.alpine.resolve()), ok=False)
        bad = home / 'invalid-rootfs'
        bad.write_bytes(b'not trusted')
        run('env', 'create', 'failed', '--archive', str(bad), ok=False)
        assert not (home / 'environments/failed').exists()
        # Existing stores can be copied into the registry without changing them.
        run('env', 'import', 'imported', str(home / 'environments/alpine/store'))
        assert 'persistent' in start('imported', 'cat /root/check').stdout
        shared = home / 'shared'
        shared.mkdir()
        run('env', 'configure', 'alpine', '--mount', '/work=' + str(shared))
        start('alpine', 'echo shared > /work/hello')
        assert (shared / 'hello').read_text().strip() == 'shared'
        child = spawn('alpine', 'echo saved > /root/stop-check; sync; echo READY; sleep 300')
        try:
            run('env', 'backup', 'alpine', str(home / 'busy.tar.gz'), ok=False)
            run('env', 'delete', 'alpine', '--yes', ok=False)
            run('env', 'start', 'alpine', '--no-windows', '/bin/true', ok=False)
            run('env', 'stop', 'alpine')
            child.wait(timeout=20)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
        assert 'saved' in start('alpine', 'cat /root/stop-check').stdout
        archive = home / 'backup.tar.gz'
        run('env', 'backup', 'alpine', str(archive))
        run('env', 'backup', 'alpine', str(archive), ok=False)
        run('env', 'restore', 'restored', str(archive))
        assert 'persistent' in start('restored', 'cat /root/check').stdout
        config = json.loads((home / 'environments/restored/config.json').read_text())
        assert config['mounts'] == []
        corrupted = home / 'corrupt.tar.gz'
        payload = bytearray(archive.read_bytes())
        payload[-5] ^= 0xff
        corrupted.write_bytes(payload)
        run('env', 'restore', 'corrupt', str(corrupted), ok=False)
        assert not (home / 'environments/corrupt').exists()
        traversal = home / 'traversal.tar.gz'
        with tarfile.open(traversal, 'w:gz') as tar:
            h = tarfile.TarInfo('../escaped')
            h.size = 1
            tar.addfile(h, io.BytesIO(b'x'))
        run('env', 'restore', 'traversal', str(traversal), ok=False)
        assert not (home.parent / 'escaped').exists()
        # Both ordinary termination and an uncatchable crash release the store.
        for sig in [signal.SIGTERM, signal.SIGHUP, signal.SIGKILL]:
            child = spawn('alpine', 'echo crash-safe > /root/crash-check; sync; echo READY; sleep 300')
            child.send_signal(sig)
            child.wait(timeout=20)
            assert 'crash-safe' in start('alpine', 'cat /root/crash-check').stdout
        master, slave = pty.openpty()
        original_terminal = termios.tcgetattr(slave)
        with (home / 'pty.stdout').open('w') as out, (home / 'pty.stderr').open('w') as err:
            child = subprocess.Popen([runner, 'env', 'start', 'alpine', '--no-windows', '/bin/sh', '-ec', 'echo READY; sleep 300'], env=env, stdin=slave, stdout=out, stderr=err)
            deadline = time.monotonic() + 20
            while 'READY' not in (home / 'pty.stdout').read_text():
                assert child.poll() is None and time.monotonic() < deadline
                time.sleep(.05)
            assert termios.tcgetattr(slave) != original_terminal
            child.send_signal(signal.SIGTERM)
            child.wait(timeout=20)
            restored_terminal = termios.tcgetattr(slave)
            # Darwin sets PENDIN while switching back to canonical input; it
            # is transient kernel queue state, not a lost terminal setting.
            restored_terminal[3] &= ~termios.PENDIN
            original_terminal[3] &= ~termios.PENDIN
            assert restored_terminal == original_terminal
            assert child.returncode == 143
        os.close(slave)
        os.close(master)
        run('env', 'check', 'alpine')
        run('env', 'delete', 'restored', '--yes')
        assert (shared / 'hello').exists()
        # A future configuration fails without replacing the current data.
        future = home / 'environments/debian/config.json'
        conf = json.loads(future.read_text())
        conf['schema'] = 999
        future.write_text(json.dumps(conf))
        run('env', 'start', 'debian', '--no-windows', '/bin/true', ok=False)
        assert any(x['state'] == 'needs attention' for x in json.loads(run('env', 'list').stdout))
        if a.soak_seconds:
            child = spawn('alpine', f'echo READY; end=$(( $(date +%s) + {a.soak_seconds} )); while [ $(date +%s) -lt $end ]; do echo alive > /root/soak; cat /root/soak > /dev/null; sleep 1; done; sync')
            child.wait(timeout=a.soak_seconds + 60)
            assert child.returncode == 0
            assert 'alive' in start('alpine', 'cat /root/soak').stdout
    a.output.joinpath('results.json').write_text(json.dumps({'runner_sha256': hashlib.sha256(a.runner.read_bytes()).hexdigest(), 'cases': rows}, indent=2) + '\n')
    print(f'{len(rows)} operations passed')


if __name__ == '__main__':
    main()
