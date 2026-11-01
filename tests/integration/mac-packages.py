#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Network-enabled real-HVF Debian tool setup and development acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner', type=Path, required=True)
    p.add_argument('--debian', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    rows = []
    class DebugDirectory(tempfile.TemporaryDirectory):
        def __exit__(self, kind, *details):
            if kind:
                saved = a.output / ('failed-home-' + str(time.time_ns()))
                Path(self.name).rename(saved)
                print('Retained failed test environment:', saved, flush=True)
            return super().__exit__(kind, *details)
    with DebugDirectory(prefix='trames-packages-') as temporary:
        home = Path(temporary)
        env = dict(os.environ, TRAMES_HOME=str(home))

        def run(*args, ok=True, timeout=1800):
            start = time.monotonic()
            output = a.output / ('step-%02d.stdout' % (len(rows)+1))
            errors = a.output / ('step-%02d.stderr' % (len(rows)+1))
            print('START', args[:4], flush=True)
            with output.open('w') as out, errors.open('w') as err:
                r = subprocess.run([str(a.runner.resolve()), *args], env=env, text=True,
                                   stdout=out, stderr=err, timeout=timeout)
            r.stdout = output.read_text()
            r.stderr = errors.read_text()
            rows.append(dict(args=list(args), code=r.returncode, seconds=time.monotonic()-start,
                             stdout=r.stdout, stderr=r.stderr))
            (a.output / 'progress.json').write_text(json.dumps(rows, indent=2)+'\n')
            print(args[:4], r.returncode, round(rows[-1]['seconds'], 2), flush=True)
            assert (r.returncode == 0) == ok, rows[-1]
            return r

        def shell(name, script):
            return run('env', 'start', name, '--no-windows', '--no-pty', '--init', 'shell',
                       '--', '/bin/sh', '-ec', script)

        base = ['--distro', 'debian', '--archive', str(a.debian.resolve()), '--cpus', '4']
        run('env', 'create', 'minimal', *base, '--packages', 'minimal', '--network', 'off')
        shell('minimal', 'test -x /usr/bin/apt-get; ! command -v curl; ! command -v gcc')
        run('env', 'create', 'invalid', *base, '--packages', 'typo', ok=False)
        assert not (home / 'environments/invalid').exists()
        run('env', 'create', 'offline', *base, '--network', 'off', ok=False, timeout=180)
        assert not (home / 'environments/offline').exists()
        assert not list((home / 'environments').glob('.offline-*'))
        # Default selection is deliberate: no --packages option here.
        run('env', 'create', 'daily', *base)
        result = shell('daily', '''for tool in curl wget git ssh nano vi less ps killall ip ping dig nc rsync unzip zip xz file; do command -v "$tool"; done
test -s /etc/ssl/certs/ca-certificates.crt
test -r /usr/share/bash-completion/bash_completion
test -r /usr/share/zoneinfo/Asia/Hong_Kong
! command -v gcc
curl --fail --max-time 30 -sS https://deb.debian.org/debian/README -o /root/debian-readme
test -s /root/debian-readme; head -n 2 /root/debian-readme
git --version; ssh -V
''')
        assert 'git version' in result.stdout
        run('env', 'configure', 'daily', '--network', 'off')
        assert 'already installed' in run('env', 'packages', 'daily').stdout
        run('env', 'configure', 'daily', '--network', 'full')
        run('env', 'packages', 'daily', 'development')
        result = shell('daily', '''mkdir -p /root/tool-test; cd /root/tool-test
cat > hello.c <<'C'
#include <stdio.h>
int main(void) { puts("C compiler passed"); return 0; }
C
cat > hello.cpp <<'CXX'
#include <iostream>
int main() { std::cout << "C++ compiler passed" << std::endl; }
CXX
cc hello.c -o hello-c; ./hello-c
c++ hello.cpp -o hello-cxx; ./hello-cxx
python3 -m venv venv
venv/bin/python -c 'import ssl, sqlite3; print("Python venv passed", ssl.OPENSSL_VERSION)'
venv/bin/pip --version
printf 'cmake_minimum_required(VERSION 3.20)\\nproject(hello C)\\nadd_executable(hello hello.c)\\n' > CMakeLists.txt
cmake -S . -B build -G Ninja; cmake --build build; ./build/hello
pkg-config --version
''')
        for expected in ['C compiler passed', 'C++ compiler passed', 'Python venv passed']:
            assert expected in result.stdout
        run('env', 'configure', 'daily', '--network', 'off')
        assert 'already installed' in run('env', 'packages', 'daily', 'development').stdout
        run('env', 'check', 'daily')
    record = dict(runner_sha256=hashlib.sha256(a.runner.read_bytes()).hexdigest(), cases=rows)
    (a.output / 'results.json').write_text(json.dumps(record, indent=2)+'\n')
    print(f'{len(rows)} operations passed', flush=True)


if __name__ == '__main__':
    main()
