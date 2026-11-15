#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Same-source fio on native macOS, Trames private objects and hostfs.

All paths belong to a dedicated output directory and isolated TRAMES_HOME.
Buffered throughput, per-write durability and unsupported direct I/O are reported
separately. Never use cached results as a physical SSD throughput estimate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import time

CASES = [
    ('seq-read-1m', 'read', '1MiB', 1, {}),
    ('seq-write-1m', 'write', '1MiB', 1, {'end_fsync': 1}),
    ('rand-read-4k', 'randread', '4KiB', 1, {}),
    ('rand-write-4k', 'randwrite', '4KiB', 1, {'end_fsync': 1}),
    ('rand-read-4k-4jobs', 'randread', '4KiB', 4, {}),
    ('rand-write-4k-4jobs', 'randwrite', '4KiB', 4, {'end_fsync': 1}),
    ('write-4k-fsync', 'write', '4KiB', 1, {'fsync': 1, 'end_fsync': 1}),
    ('write-4k-fdatasync', 'write', '4KiB', 1, {'fdatasync': 1, 'end_fsync': 1}),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('runner', 'native', 'guest', 'rootfs', 'output'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--runtime', type=int, default=5)
    parser.add_argument('--keep-data', action='store_true')
    parser.add_argument('--cases', nargs='+', choices=[case[0] for case in CASES],
                        help='Run only affected workloads; default is the full matrix')
    args = parser.parse_args()
    cases = [case for case in CASES if not args.cases or case[0] in args.cases]
    base = args.output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(base).free < 8 * 1024**3:
        raise SystemExit('Need at least 8 GiB free for this bounded test')
    registry = base / 'registry'
    exchange = base / 'exchange'
    exchange.mkdir(exist_ok=True)
    guest_binary = exchange / 'fio-linux'
    if args.guest.resolve() != guest_binary:
        shutil.copy2(args.guest, guest_binary)
    guest_binary.chmod(0o755)
    native = str(args.native.resolve())
    runner = str(args.runner.resolve())
    env = dict(os.environ, TRAMES_HOME=str(registry))
    setup = []

    def control(*command):
        result = subprocess.run([runner, *command], env=env, capture_output=True,
                                text=True, timeout=120)
        setup.append(dict(command=command, code=result.returncode,
                          stdout=result.stdout, stderr=result.stderr))
        (base / 'setup.json').write_text(json.dumps(setup, indent=2) + '\n')
        if result.returncode:
            raise RuntimeError(setup[-1])
        return result.stdout

    def guest(*command):
        return control('-e', 'fio-bench', 'exec', '--', *command)

    if not registry.exists():
        control('config', '--global', 'set', 'defaults.resources.cpus', 'all',
                'defaults.resources.memory', 'auto', 'defaults.network.mode', 'none',
                'defaults.display.windows', 'false', 'defaults.components.vulkan', 'false',
                'defaults.components.vaapi', 'false', 'defaults.components.audio', 'false',
                'defaults.components.audio_input', 'false', 'defaults.interop.clipboard.enabled', 'false')
        control('env', 'create', 'fio-bench', '--distro', 'debian', '--archive',
                str(args.rootfs.resolve()), '--packages', 'minimal')
        control('env', 'grant', 'fio-bench', 'clipboard', 'off')
        control('env', 'share', 'fio-bench', 'fio-test', str(exchange), '/mnt/fio', 'rw')
    guest('/bin/sh', '-ec', 'mkdir -p /opt /var/tmp/fio-private /mnt/fio/data; '
          'cp /mnt/fio/fio-linux /opt/fio; chmod 755 /opt/fio')
    host_data = base / 'native-data'
    host_data.mkdir(exist_ok=True)
    paths = {'host': str(host_data), 'private': '/var/tmp/fio-private', 'shared': '/mnt/fio/data'}
    prefix = {key: ([native] if key == 'host' else
                    [runner, '-e', 'fio-bench', 'exec', '--', '/opt/fio']) for key in paths}
    common = ['--ioengine=psync', '--thread=1', '--iodepth=1', '--direct=0',
              '--clocksource=clock_gettime', '--kb_base=1000', '--fallocate=none', '--invalidate=0',
              '--group_reporting=1', '--randrepeat=1', '--randseed=20261115',
              '--norandommap=1', '--output-format=json', '--eta=never',
              '--percentile_list=50:95:99:99.9']
    records = []
    report = dict(complete=False, date='2026-11-15', fio_version='3.43',
                  build_options=['--disable-native','--disable-shm'],
                  scope='buffered cached file I/O and explicit sync; not SSD throughput',
                  parameters=dict(engine='psync', depth=1, runtime=args.runtime,
                                  ramp_seconds=1, repeats=args.repeats, kb_base=1000,
                                  single_file_mib=256, parallel_file_mib=64,
                                  jobs=sorted({case[3] for case in cases}), clock='clock_gettime',
                                  cases=[case[0] for case in cases]),
                  versions={}, hashes={}, direct_probes={}, records=records)
    for label, path in [('native', args.native), ('guest', args.guest), ('runner', args.runner)]:
        report['hashes'][label] = hashlib.sha256(path.read_bytes()).hexdigest()
    report['environment'] = json.loads(control('-e', 'fio-bench', 'config', 'show', '--json'))
    report['versions']['host'] = subprocess.check_output([native, '--version'], text=True).strip()
    report['versions']['guest'] = guest('/opt/fio', '--version').strip()
    for command in [('sysctl', ['/usr/sbin/sysctl', '-n', 'machdep.cpu.brand_string', 'hw.logicalcpu', 'hw.memsize']),
                    ('macos', ['/usr/bin/sw_vers']), ('load_start', ['/usr/bin/uptime'])]:
        report[command[0]] = subprocess.check_output(command[1], text=True).strip()

    def execute(variant, name, options, allow_error=False):
        command = prefix[variant] + (['--disk_util=0'] if variant != 'host' else []) + common + ['--directory=' + paths[variant], '--name=' + name] + options
        begin = time.monotonic()
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=60)
        wall = time.monotonic() - begin
        (base / (variant + '-' + name + '.json')).write_text(result.stdout)
        (base / (variant + '-' + name + '.stderr')).write_text(result.stderr)
        try:
            data = json.loads(result.stdout[result.stdout.index('{'):])
        except (ValueError, json.JSONDecodeError):
            if allow_error:
                return dict(code=result.returncode, stderr=result.stderr, stdout=result.stdout)
            raise RuntimeError((command, result.returncode, result.stdout, result.stderr))
        errors = [job['error'] for job in data['jobs']]
        if not allow_error and (result.returncode or any(errors)):
            raise RuntimeError((command, result.returncode, errors, result.stderr, result.stdout[:2000]))
        return dict(data=data, wall=wall, code=result.returncode, errors=errors)

    try:
        for variant in paths:
            for count, size, stem in [(1, '256MiB', 'single'), (4, '64MiB', 'parallel')]:
                if count == 4 and not any(case[3] == 4 for case in cases):
                    continue
                execute(variant, 'prepare-' + stem,
                        ['--filename_format=' + stem + '.$jobnum', '--rw=write', '--bs=1MiB',
                         '--size=' + size, '--numjobs=' + str(count), '--end_fsync=1'])
            # Private/shared reject O_DIRECT today. Do not silently relabel cached I/O.
            report['direct_probes'][variant] = execute(variant, 'probe-direct',
                ['--filename=single.0', '--rw=read', '--bs=4KiB', '--size=1MiB',
                 '--io_size=1MiB', '--numjobs=1', '--direct=1'], allow_error=True)
        for repeat in range(1, args.repeats + 1):
            jobs = [(variant, case) for variant in paths for case in cases]
            random.Random(20261115 + repeat).shuffle(jobs)
            for variant, (case, rw, bs, count, extra) in jobs:
                if shutil.disk_usage(base).free < 6 * 1024**3:
                    raise RuntimeError('Stopped before consuming low free disk space')
                name = case + '-' + str(repeat)
                options = ['--filename_format=' + ('parallel' if count == 4 else 'single') + '.$jobnum',
                           '--rw=' + rw, '--bs=' + bs, '--size=' + ('64MiB' if count == 4 else '256MiB'),
                           '--numjobs=' + str(count), '--time_based=1', '--runtime=' + str(args.runtime),
                           '--ramp_time=1'] + ['--%s=%s' % item for item in extra.items()]
                measured = execute(variant, name, options)
                job = measured['data']['jobs'][0]
                direction = job['read'] if 'read' in rw else job['write']
                assert direction['io_bytes'] > 0 and direction['runtime'] >= args.runtime * 950
                assert direction['io_bytes'] == direction['total_ios'] * (4096 if bs == '4KiB' else 1048576), (case,direction)
                row = dict(variant=variant, case=case, repeat=repeat,
                           mib_s=direction['bw_bytes'] / 1024**2, iops=direction['iops'],
                           latency_mean_us=direction['lat_ns']['mean'] / 1000,
                           clat_p99_us=direction['clat_ns']['percentile']['99.000000'] / 1000,
                           sync=job.get('sync'), wall_seconds=measured['wall'],
                           usr_cpu=job['usr_cpu'], sys_cpu=job['sys_cpu'],
                           io_bytes=direction['io_bytes'])
                records.append(row)
                (base / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
                print(json.dumps(row), flush=True)
        report['medians'] = []
        for case, *_ in cases:
            for variant in paths:
                rows = [row for row in records if row['case'] == case and row['variant'] == variant]
                report['medians'].append(dict(variant=variant, case=case, **{
                    key: statistics.median(row[key] for row in rows) for key in
                    ('mib_s', 'iops', 'latency_mean_us', 'clat_p99_us', 'wall_seconds')},
                    mib_s_range=[min(row['mib_s'] for row in rows), max(row['mib_s'] for row in rows)]))
        report['load_end'] = subprocess.check_output(['/usr/bin/uptime'], text=True).strip()
        report['complete'] = True
    finally:
        (base / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
        if report['complete'] and not args.keep_data:
            guest('/bin/sh', '-ec', 'rm -rf /var/tmp/fio-private /mnt/fio/data')
            shutil.rmtree(host_data)
        control('env', 'stop', 'fio-bench')
    print('Completed %d fio measurements' % len(records), flush=True)


if __name__ == '__main__':
    main()
