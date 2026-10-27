#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run original ARM64 musl and measure live hardware guest reclamation."""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import select
import struct
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runner', type=Path, required=True)
parser.add_argument('--fixture', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
libc = ctypes.CDLL(None, use_errno=True)
libc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
libc.proc_pid_rusage.restype = ctypes.c_int

def usage(pid):
    data = ctypes.create_string_buffer(1024)
    assert libc.proc_pid_rusage(pid, 2, data) == 0, ctypes.get_errno()
    rss, physical = struct.unpack_from('QQ', data.raw, 64)
    return dict(host_resident_bytes=rss, host_physical_footprint=physical)

report = dict(passed=False, runner_sha256=hashlib.sha256(args.runner.read_bytes()).hexdigest(),
              fixture_sha256=hashlib.sha256(args.fixture.read_bytes()).hexdigest(), samples=[],
              scope='Actual EL0 hardware guest, unchanged static musl; host RSS and physical footprint sampled while the guest waits on stdin. RSS checks resident mapping release because preserved file-backed guest RAM is not fully charged to physical footprint. No process restart between allocation cycles.')
try:
    with (args.output / 'stderr.log').open('wb') as error:
        p = subprocess.Popen([str(args.runner.resolve()), '--metrics', '--timeout', '90',
                              str(args.fixture.resolve()), '--markers'], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=error, bufsize=0)
        lines = []
        try:
            deadline = time.monotonic() + 100
            high = None
            while p.poll() is None:
                assert time.monotonic() < deadline, 'hardware memory guest timed out'
                ready, _, _ = select.select([p.stdout], [], [], min(5, max(0, deadline - time.monotonic())))
                if not ready:
                    continue
                line = p.stdout.readline()
                if not line:
                    break
                lines.append(line)
                phase = line.decode().strip()
                if phase in ['BASELINE', 'POPULATED', 'DISCARDED', 'UNMAPPED']:
                    sample = dict(phase=phase, **usage(p.pid))
                    report['samples'].append(sample)
                    charge = sample['host_resident_bytes']
                    print(phase, round(charge / 1048576, 2), 'MiB RSS', flush=True)
                    if phase == 'POPULATED':
                        high = charge
                        # RSS counts multiple aliases of one shared file page.
                        # Keep the private-footprint bound, and use RSS only
                        # for the resident-mapping release check below.
                        physical = sample['host_physical_footprint']
                        assert physical - report['samples'][0]['host_physical_footprint'] < 96 << 20, ('premature population or duplicate private RAM', physical)
                    elif phase == 'DISCARDED':
                        assert high is not None and high - charge > 48 << 20, ('resident mappings stayed allocated', high, charge)
                    p.stdin.write(b'x')
                    p.stdin.flush()
            tail, _ = p.communicate(timeout=10)
            lines.append(tail)
        finally:
            if p.poll() is None:
                p.kill()
                p.wait()
        (args.output / 'stdout.log').write_bytes(b''.join(lines))
        assert p.returncode == 0, (p.returncode, (args.output / 'stderr.log').read_text())
        assert len(report['samples']) == 10, report
        assert b'private file COW' in b''.join(lines)
        report['passed'] = True
finally:
    (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
