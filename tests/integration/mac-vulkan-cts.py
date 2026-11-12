#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run original Linux Vulkan CTS in a dedicated Trames store, preserving QPA results.

No CTS sources or case outcomes are rewritten. Incomplete cases are recorded as
crashes/timeouts and remaining cases restart in a fresh runner. This is a local
diagnostic harness, not a conformance submission or a portability waiver list.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tarfile
import time
import xml.etree.ElementTree as ET


def parse_qpa(path):
    records = []
    active = None
    xml = []
    if not path.exists():
        return records, active
    for line in path.read_text(errors='replace').splitlines():
        if line.startswith('#beginTestCaseResult '):
            active = line.split(' ', 1)[1].strip()
            xml = []
        elif line == '#endTestCaseResult' and active:
            node = ET.fromstring('\n'.join(xml))
            result = node.find('Result')
            records.append({'case': active, 'status': result.attrib['StatusCode'],
                            'message': ''.join(result.itertext()).strip()})
            active = None
        elif line.startswith('#terminateTestCaseResult') and active:
            records.append({'case': active, 'status': line.split(' ', 1)[1].strip(),
                            'message': 'CTS terminated the active test case'})
            active = None
        elif active:
            xml.append(line)
    return records, active


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('runner', 'store', 'cts', 'data', 'libs', 'caselist', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--cpus', type=int, default=12)
    parser.add_argument('--batch-size', type=int, default=32)
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--minimum-free-gib', type=int, default=140)
    args = parser.parse_args()
    if min(args.cpus, args.batch_size, args.timeout) < 1:
        parser.error('positive CPU count, batch size and timeout required')
    marker = args.store.with_name(args.store.name + '.cts-owner.json')
    if not marker.is_file() or json.loads(marker.read_text()).get('purpose') != 'isolated-vulkan-cts':
        parser.error('store must have an explicit isolated-vulkan-cts ownership marker')
    for path in (args.runner, args.cts, args.caselist, args.store / 'manifest',
                 args.libs / 'libtrames_vulkan_icd.so', args.libs / 'libvulkan.so.1', args.libs / 'trames.json'):
        if not path.is_file():
            parser.error('missing input: ' + str(path))
    if not (args.data / 'vulkan').is_dir():
        parser.error('--data must contain the original vulkan resource directory')
    cases = [s.strip() for s in args.caselist.read_text().splitlines() if s.strip()]
    if not cases or len(set(cases)) != len(cases) or not all(re.fullmatch(r'dEQP-VK\.[A-Za-z0-9_.-]+', s) for s in cases):
        parser.error('expected unique original dEQP-VK case names')
    args.output.mkdir(parents=True, exist_ok=False)
    sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    report = {'scope': 'Selected original CTS cases; not a full CTS pass or certification',
              'started_unix': time.time(), 'cpus': args.cpus, 'requested': len(cases),
              'hashes': {name: sha(path) for name, path in {
                  'runner': args.runner, 'cts': args.cts, 'caselist': args.caselist,
                  'icd': args.libs / 'libtrames_vulkan_icd.so',
                  'loader': args.libs / 'libvulkan.so.1'}.items()},
              'cases': [], 'attempts': [], 'infrastructure_error': None}
    runner = str(args.runner.resolve())
    base = [runner, '--storage', str(args.store.resolve()),
            '--network', 'off', '--memory-mib', 'auto', '--cpus', str(args.cpus),
            '--no-pty', '--no-windows', '--vulkan', '--timeout', str(args.timeout)]
    pending = list(cases)
    def save():
        report['counts'] = dict(Counter(r['status'] for r in report['cases']))
        report['remaining'] = len(pending)
        (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
    try:
        # Deploy once into private guest storage. This also keeps CTS execution
        # independent of shared-folder behavior and avoids source-tree mounts.
        fingerprint = hashlib.sha256()
        for directory in (args.libs, args.data / 'vulkan'):
            for path in sorted(directory.rglob('*')):
                if path.is_file():
                    fingerprint.update(str(path.relative_to(directory)).encode())
                    fingerprint.update(bytes.fromhex(sha(path)))
        payload_key = [report['hashes']['cts'], fingerprint.hexdigest()]
        owner = json.loads(marker.read_text())
        if owner.get('payload') != payload_key:
            payload = args.output / 'payload.tar'
            with tarfile.open(payload, 'w') as archive:
                archive.add(args.cts, arcname='cts-binary/' + args.cts.name)
                archive.add(args.libs, arcname='cts-libs')
                archive.add(args.data / 'vulkan', arcname='cts-data/vulkan')
            with (args.output / 'deployment.log').open('wb') as log:
                subprocess.run([runner, '--storage', str(args.store.resolve()),
                                '--network', 'off', '--no-pty', '--no-windows', '--timeout', '120',
                                '--exec-file', '/tmp/cts-payload.tar=' + str(payload.resolve()),
                                '/bin/sh', '-ec', 'tar -xf /tmp/cts-payload.tar -C /; rm /tmp/cts-payload.tar; mkdir -p /cts-run'],
                               stdout=log, stderr=subprocess.STDOUT, check=True, timeout=135)
            owner['payload'] = payload_key
            marker.write_text(json.dumps(owner, indent=2) + '\n')
            payload.unlink()
        report['payload_key'] = payload_key
        while pending:
            if shutil.disk_usage(args.output).free < args.minimum_free_gib * 1024**3:
                raise RuntimeError('stopped at minimum free-disk budget')
            if sum(p.stat().st_size for p in args.output.iterdir() if p.is_file()) > 2 * 1024**3:
                raise RuntimeError('stopped at 2 GiB diagnostic-log budget')
            n = len(report['attempts'])
            batch = pending[:args.batch_size]
            name = f'attempt-{n:04d}'
            (args.output / (name + '.txt')).write_text('\n'.join(batch) + '\n')
            qpa = args.output / (name + '.qpa')
            guest_name = hashlib.sha256(str(args.output.resolve()).encode()).hexdigest()[:12] + '-' + name
            command = ['/cts-binary/' + args.cts.name,
                       '--deqp-archive-dir=/cts-data',
                       '--deqp-caselist-file=/cts-run/' + guest_name + '.txt',
                       '--deqp-log-filename=/cts-run/' + guest_name + '.qpa',
                       '--deqp-log-images=disable', '--deqp-log-shader-sources=disable',
                       '--deqp-log-flush=enable', '--deqp-watchdog=enable',
                       '--deqp-watchdog-total-time-limit=60',
                       '--deqp-watchdog-interval-time-limit=30']
            trace = ' TRAMES_VULKAN_TRACE=1' if os.environ.get('TRAMES_VULKAN_TRACE') else ''
            shell = ('cd /cts-run; export LD_LIBRARY_PATH=/cts-libs '
                     'VK_DRIVER_FILES=/cts-libs/trames.json' + trace + '; exec ' + shlex.join(command))
            started = time.monotonic()
            timed_out = False
            with (args.output / (name + '.stdout')).open('wb') as out, (args.output / (name + '.stderr')).open('wb') as err:
                try:
                    result = subprocess.run([*base, '--exec-file', '/cts-run/' + guest_name + '.txt=' + str((args.output / (name + '.txt')).resolve()), '/bin/sh', '-ec', shell], stdout=out, stderr=err,
                                            env=dict(os.environ, TRAMES_MAC_BUNDLED='1', MVK_CONFIG_LOG_LEVEL='1'),
                                            timeout=args.timeout + 15)
                    code = result.returncode
                except subprocess.TimeoutExpired:
                    code = None
                    timed_out = True
            with (args.output / (name + '.extract.log')).open('wb') as log:
                subprocess.run([runner, '--storage', str(args.store.resolve()), '--fs', 'get',
                                '/cts-run/' + guest_name + '.qpa', str(qpa.resolve())],
                               stdout=log, stderr=subprocess.STDOUT, timeout=30)
            records, active = parse_qpa(qpa)
            if active:
                records.append({'case': active, 'status': 'Timeout' if timed_out or code == 124 else 'Crash',
                                'message': f'Active CTS case did not finish; runner exit={code}'})
            report['attempts'].append({'name': name, 'requested': len(batch), 'exit_code': code,
                                       'host_timeout': timed_out, 'elapsed_seconds': time.monotonic() - started})
            if not records:
                raise RuntimeError('CTS produced no case result or active-case marker; inspect ' + name)
            known = set(batch)
            if any(r['case'] not in known for r in records) or len({r['case'] for r in records}) != len(records):
                raise RuntimeError('unexpected/duplicate CTS case identity')
            report['cases'].extend(records)
            done = {r['case'] for r in records}
            pending = [case for case in pending if case not in done]
            save()
            print(json.dumps({'completed': len(report['cases']), 'requested': len(cases),
                              'counts': report['counts'], 'attempt': name}), flush=True)
    except Exception as error:
        report['infrastructure_error'] = str(error)
        print('CTS infrastructure:', error, flush=True)
    finally:
        report['finished_unix'] = time.time()
        save()
    bad = {'Fail', 'InternalError', 'ResourceError', 'Crash', 'Timeout', 'DeviceLost'}
    return 1 if pending or report['infrastructure_error'] or any(r['status'] in bad for r in report['cases']) else 0


if __name__ == '__main__':
    raise SystemExit(main())
