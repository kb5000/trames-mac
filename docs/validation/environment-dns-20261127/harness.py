#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Actual HVF environment-name DNS acceptance with an existing ARM64 Linux rootfs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(a):
    out = a.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    runner = out/'runner'
    shutil.copy2(a.runner, runner)
    receipt = json.loads(a.receipt.read_text())
    assert receipt['fingerprint']['binary_sha256'] == digest(runner)
    shutil.copy2(a.receipt, out/'build-receipt.json')
    shutil.copy2(__file__, out/'harness.py')
    subprocess.run(['codesign', '--verify', '--strict', runner], check=True)
    env = dict(os.environ)
    for key in ['TRAMES_HOME', 'DISPLAY', 'WAYLAND_DISPLAY']:
        env.pop(key, None)
    rows = []

    def op(home, *args, ok=True):
        began = time.monotonic()
        r = subprocess.run([str(runner), *map(str,args)], env=env|{'TRAMES_HOME':str(home)}, capture_output=True, timeout=90)
        row = dict(argv=list(map(str,args)), code=r.returncode, stdout=r.stdout.decode(errors='replace'), stderr=r.stderr.decode(errors='replace'), seconds=time.monotonic()-began)
        rows.append(row)
        (out/'operations.json').write_text(json.dumps(rows, indent=2)+'\n')
        assert (r.returncode==0)==ok, row
        return r.stdout

    for cpus in [1,4]:
        with tempfile.TemporaryDirectory(prefix=f'hvf-{cpus}-', dir=out) as temp:
            home = Path(temp)
            try:
                op(home,'config','--global','set','defaults.components.vulkan','false','defaults.display.windows','false','defaults.resources.cpus',cpus)
                op(home,'network','create','lan','--internal')
                op(home,'network','create','other','--internal')
                for name, network in [('web','lan'),('db','lan'),('isolated','other')]:
                    op(home,'env','create',name,'--rootfs',a.rootfs)
                    op(home,'-e',name,'config','set','network.mode','bridge','network.bridge',network)
                    op(home,'env','start',name)
                def resolve(name,ok=True,peer='web'):
                    answers=[]
                    for mode in ['resolve','resolve6']:
                        answers.append(op(home,'-e',peer,'exec','--','/bin/netprobe',mode,name+'.trames',ok=ok).strip())
                    return answers
                first=resolve('db')
                assert first==resolve('DB')==resolve('db.env')
                resolve('db',False,'isolated')
                op(home,'env','stop','db')
                resolve('db',False)
                op(home,'env','rename','db','database')
                op(home,'env','start','database')
                resolve('db',False)
                assert first==resolve('database')
                op(home,'env','restart','database')
                assert first==resolve('database')
                op(home,'env','stop','database')
            finally:
                for name in ['web','db','database','isolated']:
                    subprocess.run([runner,'env','stop',name],env=env|{'TRAMES_HOME':str(home)},capture_output=True,timeout=90)
            print(f'HVF {cpus} CPUs: passed',flush=True)
    (out/'results.json').write_text(json.dumps(dict(passed=True,complete=True,runner_sha256=digest(runner),operations=len(rows),cpus=[1,4]),indent=2)+'\n')


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner',type=Path,required=True)
    p.add_argument('--receipt',type=Path,required=True)
    p.add_argument('--rootfs',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    main(p.parse_args())
