#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable real-HVF acceptance of Trames IP links and granted controls."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner',type=Path,required=True)
    p.add_argument('--alpine',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    rows=[];children=[]
    with tempfile.TemporaryDirectory(prefix='trames-links-') as temporary:
        home=Path(temporary);env=dict(os.environ,TRAMES_HOME=str(home));runner=str(a.runner.resolve())
        def record(args,result):
            rows.append(dict(args=args,code=result.returncode,stdout=result.stdout,stderr=result.stderr))
            (a.output/'commands.json').write_text(json.dumps(rows,indent=2)+'\n')
        def run(*args,code=0,timeout=25):
            r=subprocess.run([runner,*args],env=env,capture_output=True,text=True,timeout=timeout)
            record(args,r);assert r.returncode==code,rows[-1];return r.stdout
        def execute(n,script,**kw):
            return run('debug','env-run',n,'--no-pty','/bin/sh','-ec',script,**kw)
        def spawn(n,script):
            path=a.output/(n+'-'+str(len(children)))
            out=path.with_suffix('.stdout');err=path.with_suffix('.stderr')
            f=out.open('w');g=err.open('w')
            child=subprocess.Popen([runner,'debug','env-run',n,'--no-pty','/bin/sh','-ec',script],env=env,stdin=subprocess.DEVNULL,stdout=f,stderr=g,text=True)
            f.close();g.close();children.append(child);deadline=time.monotonic()+15
            while 'READY' not in out.read_text():
                assert child.poll() is None and time.monotonic()<deadline,(out.read_text(),err.read_text())
                time.sleep(.02)
            return child,out,err
        def finish(child,out,err,expected):
            child.wait(timeout=15)
            assert child.returncode==0 and expected in out.read_text(),(child.returncode,out.read_text(),err.read_text())
        def free_port():
            with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
        try:
            run('config','--global','set','defaults.resources.cpus','2','defaults.display.windows','false','defaults.components.vulkan','false','defaults.components.thunking','false')
            for n in ['a','b','c']:
                run('env','create',n,'--archive',str(a.alpine.resolve()))
            # host topology is native: separate environments see one port space.
            port=free_port()
            child,out,err=spawn('a',f'nc -l -p {port} >/root/native & echo READY; wait; cat /root/native')
            execute('b',f'nc -l -p {port}',code=1)
            with socket.create_connection(('127.0.0.1',port),timeout=3) as client:client.sendall(b'host-native');client.shutdown(socket.SHUT_WR)
            finish(child,out,err,'host-native')
            for n in ['lan','isolated']:run('network','create',n,'--internal')
            for n,network in [('a','lan'),('b','lan'),('c','isolated')]:
                run('-e',n,'config','set','network.mode','bridge','network.bridge',network)
            child,out,err=spawn('a','nc -l -p 47000 >/root/peer & echo READY; wait; cat /root/peer')
            networks=json.loads(run('network','ls','--json'));lan=next(n for n in networks if n['name']=='lan')
            uuid=json.loads(run('-e','a','config','show','--json'))['environment']['uuid'];ip='10.0.0.'+str(lan['members'][uuid])
            execute('c',f'echo isolated | nc -w 1 {ip} 47000',code=1)
            execute('b',f'printf peer-b | nc -w 2 {ip} 47000 || true')
            finish(child,out,err,'peer-b')
            # Same guest ports can be bound concurrently on separate nodes.
            ca,oa,ea=spawn('a','nc -w 8 -l -p 47001 >/root/a & echo READY; wait || true')
            cb,ob,eb=spawn('b','nc -w 8 -l -p 47001 >/root/b & echo READY; wait || true')
            run('env','stop','a');run('env','stop','b');ca.wait(timeout=10);cb.wait(timeout=10)
            # A previously blocked listener observes a newly published host port.
            run('env','grant','a','ports','on');run('-e','a','config','set','interop.ports.enabled','true')
            port=free_port();params=json.dumps(dict(host='127.0.0.1:'+str(port),guest=47002))
            child,out,err=spawn('a',"nc -l -p 47002 >/root/published & sleep .1; trames rpc port-forward ports.add '"+params+"'; echo READY; wait; cat /root/published")
            with socket.create_connection(('127.0.0.1',port),timeout=3) as client:client.sendall(b'published-live');client.shutdown(socket.SHUT_WR)
            finish(child,out,err,'published-live')
            shared=home/'shared';shared.mkdir()
            run('env','grant','a','share','project',str(shared),'rw');run('env','grant','a','mounts','on')
            run('-e','a','config','set','interop.mounts.enabled','true','interop.mounts.shares','["project"]')
            output=execute('a',"mkdir -p /mnt/control; trames rpc mount-control mounts.add '{\"share\":\"project\",\"target\":\"/mnt/control\"}'; echo original-resource >/mnt/control/check; trames rpc mount-control mounts.remove '{\"target\":\"/mnt/control\"}'; test ! -e /mnt/control/check; echo MOUNT_OK")
            assert 'MOUNT_OK' in output and (shared/'check').read_text().strip()=='original-resource'
        finally:
            for child in children:
                if child.poll() is None:
                    child.terminate()
                    try:child.wait(timeout=5)
                    except subprocess.TimeoutExpired:child.kill();child.wait()
    print('Native host sockets, cross-process bridges/isolation, port reuse, live publishing and original-resource mounts passed')


if __name__=='__main__':main()
