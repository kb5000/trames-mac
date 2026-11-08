#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Real host exec and native terminal lifecycle while a guest executor stays busy."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import socket
import struct
import subprocess
import tempfile
import time


def frame(kind, data=b''):
    return bytes([kind]) + struct.pack('!I', len(data)) + data


def exact(sock, size):
    data = bytearray()
    while len(data) < size:
        more = sock.recv(size - len(data))
        assert more, 'unexpected transport EOF'
        data.extend(more)
    return bytes(data)


class Terminal:
    def __init__(self, path, rows=30, columns=100):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(15)
        self.socket.connect(str(path))
        self.socket.sendall(b'TRAMES02' + bytes(8) + frame(1, struct.pack('!HH', rows, columns)))
        header = exact(self.socket, 17)
        assert header[:9] == b'TRAMES02\0'
        self.id = int.from_bytes(header[9:], 'big')
        self.wait(b'# ')

    def wait(self, marker):
        data = bytearray()
        while marker not in data:
            more = self.socket.recv(65536)
            assert more, 'terminal closed before marker'
            data.extend(more)
        return bytes(data)

    def command(self, command, label):
        self.socket.sendall(frame(0, (command + f"; printf '\\n{label}:%s\\n' $?\n").encode()))
        data = self.wait((label + ':0\r\n').encode())
        return data

    def close(self):
        self.socket.close()


def execute(path, command):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(15)
        client.connect(str(path.with_suffix('.exec')))
        request = json.dumps(dict(argv=['/bin/sh', '-c', command], environment=[], cwd=None, tty=False, size=None)).encode()
        client.sendall(b'TRMEXEC1' + struct.pack('!I', len(request)) + request + frame(1))
        streams = [bytearray(), bytearray()]
        while True:
            kind, size = struct.unpack('!BI', exact(client, 5))
            data = exact(client, size)
            if kind in (1, 2):
                streams[kind - 1].extend(data)
            elif kind == 3:
                assert int.from_bytes(data, 'big', signed=True) == 0, (data, streams)
                return bytes(streams[0])
            else:
                assert kind != 4, data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--alpine', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cpus', type=int, default=4)
    parser.add_argument('--baseline', action='store_true', help='record the previous implementation without requiring the removed cohort boundary')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='trames-host-lifecycle-') as temp:
        path = Path(temp) / 'console.sock'
        command = [str(args.runner.resolve()), '--rootfs', str(args.alpine.resolve()),
                   '--storage', str(Path(temp) / 'disk'), '--console-socket', str(path),
                   '--cpus', str(args.cpus), '--network', 'off', '--no-windows', '--metrics']
        clients = []
        with (args.output / 'session.log').open('wb') as log:
            guest = subprocess.Popen(command, env=dict(os.environ, TRAMES_MAC_BUNDLED='1'),
                                     stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            try:
                until = time.monotonic() + 30
                while not path.exists():
                    assert guest.poll() is None and time.monotonic() < until
                    time.sleep(.01)
                a = Terminal(path)
                clients.append(a)
                a.command('stty -echo; while :; do :; done & echo $! > /tmp/busy', 'BUSY')
                b = Terminal(path, 41, 121)
                clients.append(b)
                b.command('test "$(stty size)" = "41 121"; echo $$ > /tmp/survivor', 'SIZE')
                b.socket.sendall(frame(1, struct.pack('!HH', 42, 122)))
                b.command('test "$(stty size)" = "42 122"', 'RESIZE')
                began = time.monotonic_ns()
                for i in range(16):
                    assert execute(path, f'printf job{i}') == f'job{i}'.encode()
                elapsed = time.monotonic_ns() - began
                # Disconnect before the launcher's queued slave open completes.
                for _ in range(8):
                    with socket.socket(socket.AF_UNIX) as client:
                        client.connect(str(path))
                        client.sendall(b'TRAMES02' + bytes(8))
                        exact(client, 17)
                a.close()
                clients.remove(a)
                b.command('test $$ = "$(cat /tmp/survivor)"; kill -0 $(cat /tmp/busy)', 'SURVIVOR')
                assert execute(path, 'printf alive') == b'alive'
            finally:
                for client in clients:
                    client.close()
                guest.send_signal(signal.SIGTERM)
                try:
                    guest.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    guest.kill()
                    guest.wait()
                    raise
            assert guest.returncode in (0, 143), guest.returncode
    text = (args.output / 'session.log').read_text()
    metrics = json.loads(re.search(r'trames mac execution paths: (\{[^\n]+\})', text)[1])
    assert metrics['service_requests'] == metrics['serviced_syscalls'] == 0
    if not args.baseline and args.cpus > 1:
        assert metrics['cohort_rounds'] <= 2, metrics
    result = dict(passed=True, cpus=args.cpus, runner_sha256=hashlib.sha256(args.runner.read_bytes()).hexdigest(),
                  exec_16_elapsed_ns=elapsed, metrics=metrics,
                  checks=['live_exec', 'native_terminal_open', 'initial_size', 'live_resize',
                          'early_close_before_slave_open', 'master_close', 'busy_peer_survives', 'no_service_requests'])
    (args.output / 'results.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
