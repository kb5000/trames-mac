#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Real multi-vCPU Linux execution and preserved live-page eviction on macOS."""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import select
import struct
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--runner', type=Path, required=True)
p.add_argument('--fixture', type=Path, required=True)
p.add_argument('--rootfs', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
libc = ctypes.CDLL(None, use_errno=True)
libc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]

def usage(pid):
    data = ctypes.create_string_buffer(1024)
    assert libc.proc_pid_rusage(pid, 2, data) == 0, ctypes.get_errno()
    return dict(zip(('rss', 'physical_footprint'), struct.unpack_from('QQ', data.raw, 64)))

report = dict(passed=False, runner_sha256=hashlib.sha256(a.runner.read_bytes()).hexdigest(),
              fixture_sha256=hashlib.sha256(a.fixture.read_bytes()).hexdigest(), cases=[])
try:
    for name, cpus, resident in [('smp', 4, None), ('paging', 4, 16), ('single-paging', 1, 16)]:
        command = [str(a.runner.resolve()), '--rootfs', str(a.rootfs.resolve()),
                   '--no-pty', '--metrics', '--timeout', '120', '--cpus', str(cpus),
                   '--exec-file', '/smp-paging=' + str(a.fixture.resolve())]
        if resident:
            command += ['--resident-mib', str(resident)]
        command += ['/smp-paging', '--markers']
        case = dict(name=name, cpus=cpus, resident_target_mib=resident, samples=[], passed=False)
        report['cases'].append(case)
        began = time.monotonic()
        with (a.output / (name + '.stderr')).open('wb') as error:
            process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=error, bufsize=0)
            lines = []
            try:
                deadline = began + 130
                while process.poll() is None:
                    assert time.monotonic() < deadline, name + ' timed out'
                    if not select.select([process.stdout], [], [], 2)[0]:
                        continue
                    line = process.stdout.readline()
                    if not line:
                        break
                    lines.append(line)
                    phase = line.decode().strip()
                    if phase in ('BASELINE', 'POPULATED', 'VERIFIED', 'UNMAPPED'):
                        time.sleep(.05)
                        sample = dict(phase=phase, **usage(process.pid))
                        case['samples'].append(sample)
                        print(name, sample, flush=True)
                        process.stdin.write(b'x')
                        process.stdin.flush()
                tail, _ = process.communicate(timeout=5)
                lines.append(tail)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
        stdout = b''.join(lines).decode()
        (a.output / (name + '.stdout')).write_text(stdout)
        stderr = (a.output / (name + '.stderr')).read_text()
        case.update(exit=process.returncode, seconds=time.monotonic() - began)
        assert process.returncode == 0, (name, stdout, stderr)
        assert [s['phase'] for s in case['samples']] == ['BASELINE', 'POPULATED', 'VERIFIED', 'UNMAPPED']
        assert '8388607828' in stdout and '16777215656' in stdout and 'paged fork COW: ok' in stdout
        if cpus == 4:
            assert 'peak=4 ' in stderr, stderr
        if resident:
            assert 'page_out_bytes: 0,' not in stderr and 'page_ins: 0 ' not in stderr, stderr
        case['passed'] = True
    full, paged = report['cases'][:2]
    # The VM and its dirty 64MiB mapping are alive in both POPULATED samples.
    # Compare RSS as well as footprint: file-backed pages have different macOS
    # accounting, so footprint alone cannot demonstrate real physical eviction.
    assert full['samples'][1]['rss'] - paged['samples'][1]['rss'] > 24 << 20, report
    report['passed'] = True
finally:
    (a.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
