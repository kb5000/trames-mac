#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Network-enabled, disposable Debian systemd product acceptance on real HVF."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import termios
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--debian', type=Path)
    source.add_argument('--prepared-store', type=Path, help='reuse an existing stopped systemd store')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    runner = str(args.runner.resolve())
    records = []
    with tempfile.TemporaryDirectory(prefix='trames-systemd-') as temp:
        home = Path(temp)
        env = dict(os.environ, TRAMES_HOME=str(home))

        def run(*command, ok=True, timeout=300):
            r = subprocess.run([runner, *command], env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, timeout=timeout)
            records.append(dict(command=command, code=r.returncode, stdout=r.stdout, stderr=r.stderr))
            (args.output / 'commands.json').write_text(json.dumps(records, indent=2) + '\n')
            assert (r.returncode == 0) == ok, records[-1]
            return r

        # Rejection happens before download or publication. Old configs still
        # default to shell; installation is only triggered by explicit opt-in.
        run('env', 'create', 'invalid', '--init', 'systemd', ok=False)
        assert not (home / 'environments/invalid').exists()
        if args.prepared_store:
            run('env', 'import', 'debian', str(args.prepared_store.resolve()))
            run('env', 'configure', 'debian', '--init', 'systemd')
        else:
            run('env', 'create', 'debian', '--distro', 'debian', '--packages', 'minimal', '--init', 'systemd',
                '--archive', str(args.debian.resolve()))
        assert json.loads((home / 'environments/debian/config.json').read_text())['init'] == 'systemd'
        run('env', 'start', 'debian', '--no-windows', '/bin/true', ok=False)
        assert 'MAINTENANCE' in run('env', 'start', 'debian', '--init', 'shell', '--no-windows',
                                     '--no-pty', '/bin/echo', 'MAINTENANCE').stdout

        service = '''[Unit]
Description=Trames deployment acceptance
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh -ec 'echo TRAMES_DEPLOY_SERVICE_OK; echo persisted > /root/systemd-deploy-check'
[Install]
WantedBy=multi-user.target
'''
        notify = '''[Service]
Type=notify
NotifyAccess=all
ExecStart=/bin/sh -ec 'systemd-notify --ready; echo TRAMES_NOTIFY_OK; exec sleep infinity'
TimeoutStopSec=3s
'''
        encoded_service = base64.b64encode(service.encode()).decode()
        encoded_notify = base64.b64encode(notify.encode()).decode()
        for cpus in (1, 4):
            run('env', 'configure', 'debian', '--cpus', str(cpus), '--windows', 'off')
            master, slave = pty.openpty()
            original = termios.tcgetattr(slave)
            child = subprocess.Popen([runner, 'env', 'start', 'debian'], env=env,
                                     stdin=slave, stdout=slave, stderr=slave)
            output = bytearray()
            log = args.output / f'console-{cpus}.log'
            began = time.monotonic()

            def pump():
                if select.select([master], [], [], .05)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError:
                        pass
                    log.write_bytes(output)

            def until(marker, timeout=40):
                deadline = time.monotonic() + timeout
                while marker not in output:
                    pump()
                    assert child.poll() is None and time.monotonic() < deadline, log

            try:
                until(b'Attached terminal ')
                boot_seconds = time.monotonic() - began
                until(b'\n# ')
                install = (f"printf %s '{encoded_service}' | base64 -d > /etc/systemd/system/trames-deploy-check.service; "
                           "systemctl daemon-reload; systemctl enable --now trames-deploy-check.service; ") if cpus == 1 else ''
                command = ("set -e; stty -echo; " + install +
                           "test \"$(cat /proc/1/comm)\" = systemd; "
                           "systemctl is-active dbus.service trames-deploy-check.service; "
                           "test \"$(cat /root/systemd-deploy-check)\" = persisted; "
                           f"printf %s '{encoded_notify}' | base64 -d > /run/systemd/system/trames-notify-check.service; "
                           "systemctl daemon-reload; systemctl start trames-notify-check.service; "
                           "systemctl is-active trames-notify-check.service; "
                           "journalctl --no-pager -u trames-deploy-check.service -o cat | grep TRAMES_DEPLOY_SERVICE_OK; "
                           "journalctl --no-pager -u trames-notify-check.service -o cat | grep TRAMES_NOTIFY_OK; "
                           "systemctl stop trames-notify-check.service; "
                           "test -z \"$(systemctl --no-pager --no-legend --plain --state=failed list-units)\"; "
                           "printf '\\nACCEPT_%s\\n' PASS\n")
                os.write(master, command.encode())
                until(b'ACCEPT_PASS')
                stop = subprocess.Popen([runner, 'env', 'stop', 'debian'], env=env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                # Keep consuming the real terminal while shutdown prints. A
                # full host PTY would otherwise block the output transport.
                deadline = time.monotonic() + 60
                while child.poll() is None and time.monotonic() < deadline:
                    pump()
                child.wait(timeout=5)
                stop_output = stop.communicate(timeout=5)
                assert child.returncode == 0 and stop.returncode == 0, (log, stop_output)
                assert b'Stopped debian' in stop_output[0], log
                assert 'trames: systemd ready' in (home / 'logs/debian.log').read_text()
                actual = termios.tcgetattr(slave)
                # Darwin records transient input retyping in PENDIN when
                # restoring canonical mode; it is not a user termios setting.
                actual[3] &= ~termios.PENDIN
                original[3] &= ~termios.PENDIN
                assert actual == original, (original, actual)
                records.append(dict(cpus=cpus, boot_seconds=boot_seconds, console=str(log), graceful_stop=True))
            finally:
                if child.poll() is None:
                    child.terminate()
                    deadline = time.monotonic() + 10
                    while child.poll() is None and time.monotonic() < deadline:
                        pump()
                    if child.poll() is None:
                        child.kill()
                    child.wait()
                os.close(master)
                os.close(slave)
                subprocess.run([runner, 'env', 'stop', 'debian'], env=env,
                               capture_output=True, timeout=90, check=True)
        # Mode changes preserve the guest disk; backups retain the selected init.
        run('env', 'backup', 'debian', str(home / 'systemd.tar.gz'))
        run('env', 'restore', 'restored', str(home / 'systemd.tar.gz'))
        assert json.loads((home / 'environments/restored/config.json').read_text())['init'] == 'systemd'
        identity = run('env', 'start', 'debian', '--init', 'shell', '--no-windows', '--no-pty', '/bin/cat', '/etc/machine-id').stdout.strip()
        clone_identity = run('env', 'start', 'restored', '--init', 'shell', '--no-windows', '--no-pty', '/bin/cat', '/etc/machine-id').stdout.strip()
        assert len(identity) == 32 and len(clone_identity) == 32 and identity != clone_identity
        run('env', 'configure', 'debian', '--init', 'shell')
        assert 'persisted' in run('env', 'start', 'debian', '--no-windows', '--no-pty',
                                  '/bin/cat', '/root/systemd-deploy-check').stdout
    (args.output / 'result.json').write_text(json.dumps(dict(
        runner_sha256=hashlib.sha256(Path(runner).read_bytes()).hexdigest(), checks=records), indent=2) + '\n')
    print('PASS: optional init, original services/journal/notify, persistent deployment, graceful stop, mode switch and backup')


if __name__ == '__main__':
    main()
