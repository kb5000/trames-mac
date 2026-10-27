#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Paired Apple Clang CoreMark on macOS and the full HVF Linux session."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner', type=Path, required=True)
    p.add_argument('--rootfs', type=Path, required=True)
    p.add_argument('--source', type=Path, required=True, help='Unmodified pinned upstream checkout')
    p.add_argument('--sysroot', type=Path, required=True, help='Linux AArch64 static musl headers and libraries')
    p.add_argument('--builtins', type=Path, required=True, help='Linux AArch64 libgcc.a or compiler-rt builtins archive')
    p.add_argument('--linker', type=Path, required=True, help='LLVM rust-lld supporting -flavor gnu')
    p.add_argument('--cc', default='/usr/bin/clang')
    p.add_argument('--threads', type=int, nargs='+', default=[1, 4])
    p.add_argument('--repeats', type=int, default=3)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--environment-note', default='')
    a = p.parse_args()
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    assert a.repeats >= 3 and all(n in (1, 4) for n in a.threads)
    a.output = a.output.resolve()
    a.source = a.source.resolve()
    a.sysroot = a.sysroot.resolve()
    a.runner = a.runner.resolve()
    a.rootfs = a.rootfs.resolve()
    a.output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads(Path(__file__).with_name('coremark-upstream.json').read_text())
    for name, digest in manifest['files'].items():
        assert sha(a.source / name) == digest, name
    # The algorithm files are unchanged. Both binaries use this same POSIX port;
    # only its wall clock is replaced by a monotonic clock.
    port = a.output / 'port'
    port.mkdir()
    for name in ('core_portme.c', 'core_portme.h', 'core_portme_posix_overrides.h'):
        data = (a.source / 'posix' / name).read_text()
        if name == 'core_portme.c':
            old = 'clock_gettime(CLOCK_REALTIME, _t)'
            assert data.count(old) == 1
            data = data.replace(old, 'clock_gettime(CLOCK_MONOTONIC, _t)')
        (port / name).write_text(data)
    flags = ['-O2', '-fno-lto', '-mcpu=generic', '-march=armv8-a', '-ffixed-x18', '-fomit-frame-pointer']
    sources = [a.source / n for n in ('core_main.c', 'core_list_join.c', 'core_matrix.c',
                                     'core_state.c', 'core_util.c')] + [port / 'core_portme.c']
    builds = []
    def compile_run(command):
        builds.append(command)
        subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    binaries = {}
    for threads in a.threads:
        defs = ['-DMULTITHREAD=' + str(threads), '-DUSE_PTHREAD', '-pthread', '-DITERATIONS=0',
                '-DPERFORMANCE_RUN=1', '-DTOTAL_DATA_SIZE=2000', '-DMEM_LOCATION="Heap"',
                '-DCOMPILER_FLAGS="' + ' '.join(flags) + '; USE_PTHREAD; MULTITHREAD=' + str(threads) + '"',
                '-I' + str(a.source), '-I' + str(port)]
        native = a.output / ('coremark-native-' + str(threads))
        compile_run([a.cc, '--target=arm64-apple-macos12', *flags, *defs, *map(str, sources), '-o', str(native)])
        objects = []
        for source in sources:
            obj = a.output / (source.stem + '-' + str(threads) + '.o')
            objects.append(str(obj))
            compile_run([a.cc, '--target=aarch64-linux-musl', '--sysroot=' + str(a.sysroot),
                         *flags, *defs, '-c', str(source), '-o', str(obj)])
        guest = a.output / ('coremark-linux-' + str(threads))
        lib = a.sysroot / 'lib'
        compile_run([str(a.linker), '-flavor', 'gnu', '-m', 'aarch64elf', '-static', '-o', str(guest),
                     str(lib / 'crt1.o'), str(lib / 'crti.o'), *objects, '-L' + str(lib),
                     '--start-group', '-lc', str(a.builtins.resolve()), '--end-group', str(lib / 'crtn.o')])
        binaries[threads] = {'native': native, 'hvf': guest}
    helper = a.output / 'power.m'
    helper.write_text('#import <Foundation/Foundation.h>\n#include <stdio.h>\n'
                      'int main(void) { @autoreleasepool { NSProcessInfo *p=NSProcessInfo.processInfo; '
                      'printf("{\\"thermal_state\\":%ld,\\"low_power\\":%s}\\n", '
                      '(long)p.thermalState,p.lowPowerModeEnabled?"true":"false"); } }\n')
    power = a.output / 'power'
    compile_run([a.cc, str(helper), '-framework', 'Foundation', '-o', str(power)])
    def state():
        return json.loads(subprocess.check_output([str(power)], text=True))
    report = dict(passed=False, upstream=manifest, flags=flags, compiler=subprocess.check_output([a.cc, '--version'], text=True),
                  host=platform.platform(), cpu=subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string'], text=True).strip(),
                  runner_sha256=sha(a.runner), rootfs_sha256=sha(a.rootfs), sysroot_libc_sha256=sha(a.sysroot / 'lib/libc.a'),
                  builtins_sha256=sha(a.builtins),
                  binaries={str(n): {k: sha(v) for k, v in row.items()} for n, row in binaries.items()},
                  scheduling='macOS scheduled, no affinity; serial alternating native/HVF runs',
                  environment_note=a.environment_note,
                  rows=[], summary=[])
    (a.output / 'build-commands.json').write_text(json.dumps(builds, indent=2) + '\n')
    def run(target, threads, mode, iterations, label, scored):
        seeds = ['0', '0', '0x66'] if mode == 'performance' else ['0x3415', '0x3415', '0x66']
        command = [str(binaries[threads][target]), *seeds, str(iterations)]
        if target == 'hvf':
            command = [str(a.runner), '--rootfs', str(a.rootfs), '--network', 'off', '--no-pty', '--cpus', str(threads),
                       '--memory-mib', '4096', '--timeout', '120', '--metrics',
                       '--exec-file', '/coremark=' + str(binaries[threads][target]), '/coremark', *seeds, str(iterations)]
        before = state()
        started = time.monotonic()
        result = subprocess.run(command, capture_output=True, text=True, timeout=130)
        wall = time.monotonic() - started
        after = state()
        (a.output / (label + '.stdout')).write_text(result.stdout)
        (a.output / (label + '.stderr')).write_text(result.stderr)
        assert result.returncode == 0, (label, result.stderr)
        seconds = float(re.search(r'Total time \(secs\):\s*([0-9.]+)', result.stdout)[1])
        score = float(re.search(r'Iterations/Sec\s*:\s*([0-9.]+)', result.stdout)[1])
        assert not re.search(r'ERROR!.*crc', result.stdout), result.stdout
        assert int(re.search(r'Iterations\s*:\s*(\d+)', result.stdout)[1]) == iterations * threads
        if threads > 1:
            assert re.search(r'Parallel PThreads\s*:\s*' + str(threads), result.stdout), result.stdout
        else:
            assert 'Parallel ' not in result.stdout
        if scored and target == 'hvf' and threads == 4:
            assert re.search(r'peak=4\b', result.stderr), label
        if scored:
            assert seconds >= 10 and 'Correct operation validated' in result.stdout, label
            assert ('0xe9f5' if mode == 'performance' else '0x18f2') in result.stdout
            assert all(s == {'thermal_state': 0, 'low_power': False} for s in (before, after)), (label, before, after)
        row = dict(target=target, threads=threads, mode=mode, iterations=iterations, seconds=seconds,
                   iterations_per_second=score, wall_seconds=wall, scored=scored, power_before=before, power_after=after,
                   command=command, stdout_sha256=sha(a.output / (label + '.stdout')))
        report['rows'].append(row)
        print(label, round(score, 2), 'iterations/sec;', seconds, 'sec', flush=True)
        return row
    awake = subprocess.Popen(['caffeinate', '-i', '-w', str(os.getpid())])
    try:
        for threads in a.threads:
            calibration = [run(t, threads, 'performance', 10000, 'calibration-' + t + '-' + str(threads), False)
                           for t in ('native', 'hvf')]
            iterations = math.ceil(max(r['iterations_per_second'] for r in calibration) / threads * 13 / 10000) * 10000
            for mode in ('performance', 'validation'):
                for repeat in range(a.repeats):
                    order = ('native', 'hvf') if repeat % 2 == 0 else ('hvf', 'native')
                    for target in order:
                        run(target, threads, mode, iterations, '%s-%d-%s-%d' % (target, threads, mode, repeat), True)
                medians = {t: statistics.median(r['iterations_per_second'] for r in report['rows']
                           if r['scored'] and r['threads'] == threads and r['mode'] == mode and r['target'] == t)
                           for t in ('native', 'hvf')}
                report['summary'].append(dict(threads=threads, mode=mode, median_iterations_per_second=medians,
                                              hvf_over_native=medians['hvf'] / medians['native']))
        report['passed'] = True
    finally:
        awake.terminate()
        awake.wait()
        (a.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['summary'], indent=2))


if __name__ == '__main__':
    main()
