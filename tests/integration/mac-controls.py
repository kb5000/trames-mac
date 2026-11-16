#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable real-HVF owner/guest CLI, policy and bounded byte-stream acceptance."""
import argparse,base64,hashlib,json,os,subprocess,tempfile,time,shutil
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--runner',type=Path,required=True);p.add_argument('--alpine',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);runner=str(a.runner.resolve());rows=[]
with tempfile.TemporaryDirectory(prefix='trames-control-cli-') as temporary:
    root=Path(temporary);env=dict(os.environ,TRAMES_HOME=str(root/'registry'))
    def run(*args,code=0,input=None,timeout=90):
        start=time.monotonic();r=subprocess.run([runner,*args],env=env,input=input,capture_output=True,timeout=timeout)
        rows.append(dict(argv=args,code=r.returncode,seconds=time.monotonic()-start,stdout_bytes=len(r.stdout),stderr=r.stderr.decode(errors='replace')[-3000:]))
        (a.output/'commands.json').write_text(json.dumps(rows,indent=2)+'\n')
        assert r.returncode==code,rows[-1]
        return r.stdout
    def guest(*args,**kw):return run('debug','env-run','lab','--no-windows','--no-pty',*args,**kw)
    def data(*args):return json.loads(run(*args))['data']
    run('permissions','defaults','deny','host-exec')
    run('env','create','lab','--archive',str(a.alpine.resolve()),'--cpus','2','--vulkan','false','--network','none')
    assert data('-e','lab','permissions','--json')['host-exec']['allowed'] is False
    run('-e','lab','permissions','allow','--all')
    assert all(v['allowed'] for v in data('-e','lab','permissions','--json').values())
    # Help/version/completion remain fully offline; real guest has a minimal Alpine.
    assert b'Usage: trames' in guest('/usr/bin/trames','--help')
    assert b'complete' in guest('/usr/bin/trames','completion','bash')
    assert b'trames-control' not in guest('/usr/bin/trames','--version')
    guest('/usr/bin/trames','permissions','allow','--all',code=77)
    status=json.loads(guest('/usr/bin/trames','status','--json'))['data'];assert status['platform']=='macos'
    assert b'Darwin' in guest('/usr/bin/trames','host','exec','--','/usr/bin/uname','-s')
    text='space quote" slash\\ tab\t line\nend'
    assert guest('/usr/bin/trames','host','exec','--','/usr/bin/printf','%s',text)==text.encode()
    payload=bytes(range(256))*1024
    program='import sys; data=sys.stdin.buffer.read(); sys.stdout.buffer.write(data); sys.stderr.buffer.write(bytes(range(256))); sys.exit(7)'
    result=guest('/usr/bin/trames','--json','host','exec','--','/usr/bin/python3','-c',program,input=payload,code=7)
    frames=[json.loads(line) for line in result.splitlines()]
    assert b''.join(base64.b64decode(f['data']) for f in frames if f['type']=='stdout')==payload
    assert b''.join(base64.b64decode(f['data']) for f in frames if f['type']=='stderr')==bytes(range(256))
    assert frames[-1]['exit_code']==7
    guest('/usr/bin/trames','--timeout','300ms','host','exec','--','/bin/sleep','10',code=124)
    marker=root/'cancelled-descendant'
    guest('/usr/bin/trames','--timeout','300ms','host','exec','--','/bin/sh','-c','(/bin/sleep 2; /usr/bin/touch "$0") &',str(marker),code=124)
    time.sleep(2.2);assert not marker.exists()
    assert b'Trames' in guest('/usr/bin/trames','host','cli','--','--version')
    guest('/usr/bin/trames','host','cli','--','permissions','allow','--all',code=1)
    # Owner file exchange works while stopped, preserving arbitrary bytes.
    source=root/'binary';source.write_bytes(payload)
    run('-e','lab','files','cp',str(source),'guest:/root/binary')
    offline=root/'offline';run('-e','lab','files','cp','guest:/root/binary',str(offline));assert offline.read_bytes()==payload
    assert 'binary' in data('-e','lab','files','ls','guest:/root','--json')['entries']
    shared=root/'share';shared.mkdir();(shared/'input').write_bytes(payload)
    run('env','share','lab','project',str(shared),'/mnt/project','rw')
    # Revoke dynamic-mount control: the file data view remains independently usable.
    run('-e','lab','permissions','deny','mounts')
    assert guest('/usr/bin/trames','files','path','project').strip().startswith(b'/run/trames/shares/')
    guest('/usr/bin/trames','files','cp','share:project/input','/root/shared-in')
    guest('/usr/bin/trames','files','cp','/root/shared-in','share:project/output')
    assert (shared/'output').read_bytes()==payload
    # Both owner directions use the running environment, never an independent store.
    run('-e','lab','env','start')
    try:
        run('-e','lab','files','cp',str(source),'guest:/root/live')
        live=root/'live';run('-e','lab','files','cp','guest:/root/live',str(live));assert live.read_bytes()==payload
        tree=root/'tree';tree.mkdir();(tree/'one').write_bytes(payload);(tree/'link').symlink_to('one')
        run('-e','lab','files','cp','-r',str(tree),'guest:/root/tree')
        copied=root/'tree-copy';run('-e','lab','files','cp','-r','guest:/root/tree',str(copied))
        assert (copied/'one').read_bytes()==payload and (copied/'link').readlink()==Path('one')
        run('-e','lab','files','cp','-n',str(source),'guest:/root/live')
        assert 'live' in data('-e','lab','files','ls','guest:/root','--json')['entries']
        assert data('-e','lab','status','--json')['running'] is True
        for size in [3072,7168,12288]:
            edge=root/f'edge-{size}';edge.write_bytes((bytes(range(256))*64)[:size])
            run('-e','lab','files','cp',str(edge),f'guest:/root/edge-{size}')
            back=root/f'edge-back-{size}';run('-e','lab','files','cp',f'guest:/root/edge-{size}',str(back))
            assert back.read_bytes()==edge.read_bytes()
        large=root/'large';large.write_bytes(payload*256)
        before=time.monotonic();shutil.copyfile(large,root/'native-large');native=time.monotonic()-before
        before=time.monotonic();run('-e','lab','files','cp',str(large),'guest:/root/large');upload=time.monotonic()-before
        before=time.monotonic();run('-e','lab','files','cp','guest:/root/large',str(root/'large-back'));download=time.monotonic()-before
        assert hashlib.sha256((root/'large-back').read_bytes()).digest()==hashlib.sha256(large.read_bytes()).digest()
        (a.output/'copy-performance.json').write_text(json.dumps(dict(bytes=large.stat().st_size,native_seconds=native,upload_seconds=upload,download_seconds=download),indent=2)+'\n')
    finally:run('-e','lab','env','stop')
    run('-e','lab','permissions','deny','files','host-exec')
    run('-e','lab','files','cp',str(source),'guest:/root/denied',code=77)
    guest('/usr/bin/trames','host','exec','--','/usr/bin/uname',code=77)
    run('-e','lab','permissions','deny','--all')
    guest('/usr/bin/trames','host','gui',code=77)
    guest('/usr/bin/trames','host','cli','--','--version',code=77)
    guest('/usr/bin/trames','host','open','https://example.com/',code=77)
    run('permissions','defaults','allow','--all')
    assert all(v['allowed'] for v in data('permissions','defaults','--json').values())
(a.output/'result.json').write_text(json.dumps(dict(passed=True,commands=len(rows),binary_roundtrip_bytes=len(payload),owner_copies='offline+live',guest_permissions='read-only',scoped_delegation=True),indent=2)+'\n')
print('PASS: mirrored commands, owner defaults/denials, binary stdin/stdout/stderr, exit/timeout, scoped file copies and live/offline owner transfers')
