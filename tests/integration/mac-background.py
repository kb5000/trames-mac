#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable launchd + independent guest PTY acceptance on real HVF."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time


class Terminal:
    def __init__(self, runner, env, output, name, command, rows=30, columns=100):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, 0, 0))
        self.text = bytearray()
        self.log = output / (name + '.log')
        self.child = subprocess.Popen([runner, *command], env=env, stdin=self.slave,
                                      stdout=self.slave, stderr=self.slave)
        try:
            self.wait(b'Attached terminal ')
            self.id = int(re.search(rb'Attached terminal (\d+)', self.text)[1])
            self.wait(b'# ')
        except BaseException:
            self.close()
            raise

    def pump(self):
        if select.select([self.master], [], [], .05)[0]:
            try:
                self.text.extend(os.read(self.master, 65536))
            except OSError:
                pass
            self.log.write_bytes(self.text)

    def wait(self, marker, start=0, timeout=45):
        deadline = time.monotonic() + timeout
        while marker not in self.text[start:]:
            self.pump()
            assert self.child.poll() is None and time.monotonic() < deadline, (self.log, marker, self.text[-2000:])

    def send(self, data):
        os.write(self.master, data.encode() if isinstance(data, str) else data)

    def command(self, command, label, timeout=45):
        start = len(self.text)
        self.send(command + f"; printf '\\n{label}:%d\\n' $?\n")
        end = time.monotonic() + timeout
        expression = re.compile(rb'\r?\n' + label.encode() + rb':(\d+)\r?\n')
        while True:
            self.pump()
            result = expression.search(self.text, start)
            if result:
                assert result[1] == b'0', (label, self.text[start:])
                return
            assert self.child.poll() is None and time.monotonic() < end, (label, self.log, self.text[-2000:])

    def close(self):
        if self.child.poll() is None:
            self.child.send_signal(signal.SIGTERM)
        try:
            self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.child.kill()
            self.child.wait()
        actual = termios.tcgetattr(self.slave)
        actual[3] &= ~termios.PENDIN
        self.original[3] &= ~termios.PENDIN
        os.close(self.master)
        os.close(self.slave)
        assert actual == self.original, 'host terminal was not restored'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--alpine', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    runner = str(args.runner.resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    records = []
    with tempfile.TemporaryDirectory(prefix='trames-background-') as folder:
        home = Path(folder)
        env = dict(os.environ, TRAMES_HOME=str(home))

        def run(*command):
            result = subprocess.run([runner, *command], env=env, capture_output=True, text=True, timeout=90)
            records.append(dict(command=command, code=result.returncode, stdout=result.stdout, stderr=result.stderr))
            assert result.returncode == 0, records[-1]
            return result

        run('env', 'create', 'lab', '--archive', str(args.alpine.resolve()))
        run('env', 'configure', 'lab', '--cpus', '4', '--vulkan', 'false', '--network', 'none')
        # Exercise shell argv quoting through the real launcher, not a mirrored parser.
        welcome = "WELCOME quoted ' $ % \\\nsecond line"
        argv = ['/bin/sh', '-c', 'printf "%s\\n" "$1"; exec /bin/sh -il', 'launcher', welcome]
        run('-e','lab','config','set','session.shell',json.dumps(argv))
        clients = []
        try:
            starters = [subprocess.Popen([runner, 'env', 'start', 'lab', '--detach'], env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(2)]
            for child in starters:
                output = child.communicate(timeout=90)
                assert child.returncode == 0, output
            a = Terminal(runner, env, args.output, 'a', ['-e', 'lab', 'shell'], 31, 91)
            clients.append(a)
            b = Terminal(runner, env, args.output, 'b', ['env', 'attach', 'lab'], 42, 122)
            clients.append(b)
            assert a.id != b.id
            assert welcome.replace('\n', '\r\n').encode() in a.text
            a.command('stty -echo && export ONLY_A=separate && cd /tmp && echo $$ > /tmp/shell-a && test "$(stty size)" = "31 91"', 'A')
            b.command('stty -echo && test -z "${ONLY_A+x}" && test "$(stty size)" = "42 122" && echo $$ > /tmp/shell-b', 'B')
            a.send('sleep 300\n')
            b.command('test "$(cat /tmp/shell-a)" != "$(cat /tmp/shell-b)" && echo shared > /root/term-shared', 'PARALLEL')
            a.send(b'\x03')
            a.command('test "$(cat /root/term-shared)" = shared && kill -0 $(cat /tmp/shell-b)', 'CTRL_C')
            a.send(b'\x1d')
            a.child.wait(timeout=10)
            assert a.child.returncode == 0
            aid = a.id
            a.close()
            clients.remove(a)
            a = Terminal(runner, env, args.output, 'a-reconnect', ['env', 'attach', 'lab', '--terminal', str(aid)])
            clients.append(a)
            a.command('test "$ONLY_A" = separate && test "$$" = "$(cat /tmp/shell-a)"', 'RECONNECT')
            a.send('exit\n')
            a.child.wait(timeout=10)
            a.close()
            clients.remove(a)
            b.command('kill -0 $(cat /tmp/shell-b) && test "$(cat /root/term-shared)" = shared && sync', 'SURVIVOR')
            b.close()
            clients.remove(b)
            assert json.loads(run('env', 'status', 'lab').stdout)['running']
            # Stopping and restarting the entire environment preserves its disk.
            run('env', 'stop', 'lab')
            run('env', 'start', 'lab', '--detach')
            c = Terminal(runner, env, args.output, 'cold-start', ['env', 'attach', 'lab'])
            clients.append(c)
            c.command('test "$(cat /root/term-shared)" = shared', 'PERSISTENCE')
            c.close()
            clients.remove(c)
        finally:
            import shutil
            if (home / "logs").exists():
                shutil.copytree(home / "logs", args.output / "service-logs", dirs_exist_ok=True)
            for client in clients:
                client.close()
            subprocess.run([runner, 'env', 'stop', 'lab'], env=env, capture_output=True, timeout=90, check=True)
        records.append(dict(independent_ptys=True, concurrent_start=True, configured_argv=True,
                            interrupt_isolation=True, reattach=True, last_disconnect_keeps_guest=True,
                            cold_restart=True, host_terminal_restored=True))
    (args.output / 'results.json').write_text(json.dumps(dict(runner_sha256=hashlib.sha256(args.runner.read_bytes()).hexdigest(), cases=records), indent=2) + '\n')
    print('Background lifetime, parallel PTYs, detach/reconnect and restart passed')


if __name__ == '__main__':
    main()
