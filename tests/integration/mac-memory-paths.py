#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare fixed Linux memory work on named signed Mac runners, without timing gates."""
import argparse
from functools import lru_cache
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import statistics
import subprocess
import time


def positive(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError('must be positive')
    return number


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def apply_linear(basis, value):
    result = 0
    while value:
        bit = value & -value
        result ^= basis[bit.bit_length() - 1]
        value ^= bit
    return result


@lru_cache(maxsize=None)
def peer_values(peers, iterations):
    # Independent skip-ahead oracle for the fixture's finite uint64 xorshift.
    def step(value):
        value = (value ^ (value << 13)) & ((1 << 64) - 1)
        value ^= value >> 7
        return (value ^ (value << 17)) & ((1 << 64) - 1)

    values = list(range(1, peers + 1))
    basis = [step(1 << bit) for bit in range(64)]
    while iterations:
        if iterations & 1:
            values = [apply_linear(basis, value) for value in values]
        iterations >>= 1
        if iterations:
            basis = [apply_linear(basis, value) for value in basis]
    return values


def validate(work, mode, cpus, args):
    if mode == 'peers':
        expected = dict(peers=cpus - 1, rounds=args.peer_rounds,
                        iterations=args.peer_iterations)
        values = peer_values(cpus - 1, args.peer_iterations)
        checksum = args.peer_rounds * (args.peer_rounds + 1) // 2
        for value in values:
            checksum ^= value
        expected.update(values=values, checksum=checksum)
        require(all(work.get(key) == value for key, value in expected.items()),
                f'compute/mapping result mismatch: {work}')
        require(all(work[key] >= 0 for key in ['mapping_ms', 'compute_sum_ms', 'wall_ms']),
                'negative guest duration')
        return
    threads = cpus * 2 if mode == 'reuse' else cpus
    rounds = args.reuse_rounds if mode == 'reuse' else args.churn_rounds
    pages = args.reuse_pages if mode == 'reuse' else args.churn_pages
    if mode == 'reuse':
        checksum = threads * rounds * pages * (pages + 1) // 2
    else:
        checksum = sum(rounds * (pages * i + pages * (pages - 1) // 2)
                       + pages * rounds * (rounds - 1) // 2 for i in range(threads))
    expected = dict(mode=mode, threads=threads, rounds=rounds, pages=pages,
                    checksum=checksum & ((1 << 64) - 1))
    require(all(work.get(key) == value for key, value in expected.items()),
            f'memory result mismatch: {work}')
    require(work['total_ns'] >= work['work_ns'] >= 0, 'invalid guest durations')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path, required=True, help='Static AArch64 Linux memory-paths ELF')
    parser.add_argument('--mapping-pause', type=Path, help='Static AArch64 Linux vm-checkpoint ELF for peers')
    parser.add_argument('--variant', action='append', required=True, metavar='NAME=RUNNER')
    parser.add_argument('--output', type=Path, required=True, help='New output directory')
    parser.add_argument('--cpus', type=positive, nargs='+', default=[1, 6, 12])
    parser.add_argument('--modes', nargs='+', choices=['reuse', 'churn', 'peers'], default=['reuse', 'churn'])
    parser.add_argument('--repetitions', type=positive, default=5)
    parser.add_argument('--seed', type=int, default=20261029)
    parser.add_argument('--reuse-rounds', type=positive, default=100)
    parser.add_argument('--reuse-pages', type=positive, default=2048)
    parser.add_argument('--churn-rounds', type=positive, default=60)
    parser.add_argument('--churn-pages', type=positive, default=64)
    parser.add_argument('--peer-rounds', type=positive, default=1000)
    parser.add_argument('--peer-iterations', type=positive, default=200000000)
    parser.add_argument('--timeout', type=positive, default=60, help='Per-run guest timeout in seconds')
    parser.add_argument('--power-probe', type=Path, help='Optional executable emitting thermal_state/low_power JSON')
    args = parser.parse_args()
    variants = {}
    for specification in args.variant:
        name, separator, path = specification.partition('=')
        if not separator or not re.fullmatch(r'[A-Za-z0-9_-]+', name) or not path or name in variants:
            parser.error('--variant requires a unique simple NAME=RUNNER')
        variants[name] = Path(path).resolve(strict=True)
    if len(set(args.cpus)) != len(args.cpus) or len(set(args.modes)) != len(args.modes):
        parser.error('CPU counts and modes must be unique')
    if max(args.cpus) > (16 if 'reuse' in args.modes else 32):
        parser.error('at most 32 fixture threads (reuse uses twice the CPU count)')
    if 'peers' in args.modes and args.mapping_pause is None:
        parser.error('--mapping-pause is required for peers')
    fixtures = {'memory-paths': args.fixture.resolve(strict=True)}
    if args.mapping_pause is not None:
        fixtures['mapping-pause'] = args.mapping_pause.resolve(strict=True)
    for path in fixtures.values():
        with path.open('rb') as stream:
            header = stream.read(20)
        require(header[:6] == b'\x7fELF\x02\x01' and header[18:20] == b'\xb7\x00',
                f'expected little-endian AArch64 ELF: {path}')
    args.output.mkdir(parents=True, exist_ok=False)
    report = dict(passed=False, seed=args.seed, options={key: str(value) if isinstance(value, Path) else value
                  for key, value in vars(args).items()}, rows=[],
                  runner_sha256={name: sha256(path) for name, path in variants.items()},
                  fixture_sha256={name: sha256(path) for name, path in fixtures.items()},
                  timing='No timing assertions. Guest work starts before barrier release; total_ns includes '
                         'setup/create/join but excludes final shared-array munmap. Host time includes startup/teardown.')
    awake = None

    def power():
        if args.power_probe is None:
            return None
        return json.loads(subprocess.check_output([str(args.power_probe.resolve())], timeout=5))

    try:
        report['power_before'] = power()
        if shutil.which('caffeinate'):
            awake = subprocess.Popen(['caffeinate', '-i', '-w', str(os.getpid())])
        schedule = [(repeat, cpus, mode, variant) for repeat in range(args.repetitions)
                    for cpus in args.cpus for mode in args.modes for variant in variants]
        random.Random(args.seed).shuffle(schedule)
        environment = {**os.environ, 'TRAMES_MAC_BUNDLED': '1'}
        environment.pop('TRAMES_HVF_PROFILE', None)
        for repeat, cpus, mode, variant in schedule:
            name = f'{repeat}-{mode}-{cpus}-{variant}'
            if mode == 'peers':
                guest = [str(fixtures['mapping-pause']), str(cpus - 1),
                         str(args.peer_rounds), str(args.peer_iterations)]
            else:
                threads = cpus * 2 if mode == 'reuse' else cpus
                rounds = args.reuse_rounds if mode == 'reuse' else args.churn_rounds
                pages = args.reuse_pages if mode == 'reuse' else args.churn_pages
                guest = [str(fixtures['memory-paths']), mode, str(threads), str(rounds), str(pages)]
            command = [str(variants[variant]), '--no-pty', '--no-windows', '--network', 'off',
                       '--cpus', str(cpus), '--memory-mib', 'auto', '--metrics',
                       '--timeout', str(args.timeout), *guest]
            row = dict(name=name, repeat=repeat, cpus=cpus, mode=mode, variant=variant, passed=False)
            report['rows'].append(row)
            began = time.monotonic_ns()
            try:
                result = subprocess.run(command, env=environment, capture_output=True, timeout=args.timeout + 10)
            except subprocess.TimeoutExpired as error:
                (args.output / (name + '.stdout')).write_bytes(error.stdout or b'')
                (args.output / (name + '.stderr')).write_bytes(error.stderr or b'')
                raise
            row.update(exit=result.returncode, host_ms=(time.monotonic_ns() - began) / 1000000)
            (args.output / (name + '.stdout')).write_bytes(result.stdout)
            (args.output / (name + '.stderr')).write_bytes(result.stderr)
            require(result.returncode == 0, f'{name}: exit {result.returncode}; see captured logs')
            row['work'] = json.loads(result.stdout)
            validate(row['work'], mode, cpus, args)
            for field, prefix in [('metrics', 'trames mac metrics: '),
                                  ('vm_phases_us', 'trames mac vm phases us: '),
                                  ('quiescence_bins', 'trames mac vm quiescence bins: '),
                                  ('vm_batch', 'trames mac vm batch: ')]:
                matches = [line[len(prefix):] for line in result.stderr.decode().splitlines() if line.startswith(prefix)]
                require(len(matches) == 1, f'{name}: expected one {field} report')
                row[field] = json.loads(matches[0])
            row['passed'] = True
            (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
        report['power_after'] = power()
        report['normal_power'] = (None if args.power_probe is None else
                                 all(report[key] == dict(thermal_state=0, low_power=False)
                                     for key in ['power_before', 'power_after']))
        report['medians'] = []
        for mode in args.modes:
            for cpus in args.cpus:
                for variant in variants:
                    rows = [row for row in report['rows'] if (row['mode'], row['cpus'], row['variant'])
                            == (mode, cpus, variant)]
                    median = dict(mode=mode, cpus=cpus, variant=variant,
                                  host_ms=statistics.median(row['host_ms'] for row in rows),
                                  groups=statistics.median(row['metrics']['vm_checkpoint_groups'] for row in rows),
                                  pause_ms=statistics.median(row['vm_phases_us'][0] / 1000 for row in rows),
                                  exclusive_ms=statistics.median(row['vm_phases_us'][6] / 1000 for row in rows))
                    for field in ['work_ns', 'total_ns', 'mapping_ms', 'compute_sum_ms', 'wall_ms']:
                        if field in rows[0]['work']:
                            median[field] = statistics.median(row['work'][field] for row in rows)
                    report['medians'].append(median)
        report['passed'] = True
        print(f'PASS: {len(report["rows"])} checksum-verified observations; no timing threshold')
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        if awake is not None:
            awake.terminate()
            awake.wait(timeout=5)
        (args.output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
