#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run fixed WebGL work and verify pixels in an isolated Firefox profile.

Connect to an already-running localhost Marionette port. Keep Trames visible.
One warmup precedes three measured trials by default. No graphics, security,
cache or process preference is changed. gpu_work_ms measures JavaScript time
inside draw/update/finish batches, not a hardware GPU timestamp.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import threading
import time


HTML = r'''<!doctype html><meta charset="utf-8"><title>Trames Vulkan fixed-work test</title>
<style>body{font:24px sans-serif;margin:32px}canvas{border:1px solid #888;width:512px;height:512px}button{font:inherit;padding:12px}pre{white-space:pre-wrap}</style>
<h1>Vulkan fixed-work and pixel check</h1><button id="run">Run 64 batches × 8 draws</button><span id="status">Initializing</span><p><canvas id="canvas" width="256" height="256"></canvas><pre id="results"></pre>
<script>
let gl=document.getElementById('canvas').getContext('webgl',{antialias:false,preserveDrawingBuffer:true}), run=0, lost=false;
const status=document.getElementById('status'), button=document.getElementById('run'), results=document.getElementById('results');
if(!gl){status.textContent='WebGL unavailable';button.disabled=true;}else{
document.getElementById('canvas').addEventListener('webglcontextlost',()=>lost=true);
function shader(type,source){const s=gl.createShader(type);gl.shaderSource(s,source);gl.compileShader(s);if(!gl.getShaderParameter(s,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(s));return s;}
const p=gl.createProgram();gl.attachShader(p,shader(gl.VERTEX_SHADER,'attribute vec2 position; void main(){gl_Position=vec4(position,0.0,1.0);}'));
gl.attachShader(p,shader(gl.FRAGMENT_SHADER,'precision mediump float; uniform vec4 color; void main(){gl_FragColor=color;}'));gl.linkProgram(p);
if(!gl.getProgramParameter(p,gl.LINK_STATUS))throw Error(gl.getProgramInfoLog(p));
const retained=[],payload=new Uint8Array(1024*1024);payload.fill(37);
for(let i=0;i<16;i++){let b=gl.createBuffer();retained.push(b);gl.bindBuffer(gl.ARRAY_BUFFER,b);gl.bufferData(gl.ARRAY_BUFFER,payload,gl.STATIC_DRAW);}
const vertices=new Float32Array([-1,-1,3,-1,-1,3]), buffer=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferData(gl.ARRAY_BUFFER,vertices,gl.DYNAMIC_DRAW);
gl.useProgram(p);const location=gl.getAttribLocation(p,'position'),color=gl.getUniformLocation(p,'color');gl.vertexAttribPointer(location,2,gl.FLOAT,false,0,0);gl.enableVertexAttribArray(location);gl.disable(gl.DITHER);gl.finish();status.textContent='Ready';
button.onclick=()=>{run++;button.disabled=true;let n=0,work=0,started=performance.now();
 const step=()=>{try{let began=performance.now();gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferSubData(gl.ARRAY_BUFFER,0,vertices);gl.uniform4f(color,0.25,0.5,0.75,1.0);
 for(let j=0;j<8;j++)gl.drawArrays(gl.TRIANGLES,0,3);gl.finish();work+=performance.now()-began;n++;
 status.textContent='Batch '+n+'/64';if(n<64){setTimeout(step,0);return;}
 let pixel=new Uint8Array(4);gl.readPixels(128,128,1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);gl.finish();let error=gl.getError(),expected=[64,128,191,255];
 let info=gl.getExtension('WEBGL_debug_renderer_info');let record={batches:n,draws:n*8,elapsed_ms:performance.now()-started,gpu_work_ms:work,pixel:[...pixel],pixel_ok:[...pixel].every((x,i)=>Math.abs(x-expected[i])<=1),gl_error:error,context_lost:lost,renderer:info?gl.getParameter(info.UNMASKED_RENDERER_WEBGL):gl.getParameter(gl.RENDERER)};
 let done=document.createElement('pre');done.id='done-'+run;done.textContent=JSON.stringify(record);results.appendChild(done);button.disabled=false;status.textContent='Complete';
 }catch(e){let done=document.createElement('pre');done.id='done-'+run;done.textContent=JSON.stringify({error:String(e)});results.appendChild(done);button.disabled=false;status.textContent='Error';}};setTimeout(step,0);};}
</script>'''


class Page(BaseHTTPRequestHandler):
    def do_GET(self):
        data = HTML.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(data)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port', type=int, required=True, help='localhost Marionette port')
    parser.add_argument('--output', type=Path, required=True, help='new JSONL output file')
    parser.add_argument('--rounds', type=int, default=3, help='measured trials after one warmup')
    args = parser.parse_args()
    if not 1 <= args.port <= 65535 or args.rounds < 1:
        parser.error('port must be 1..65535 and rounds must be positive')

    path = Path(__file__).with_name('benchmark-firefox-navigation.py')
    spec = importlib.util.spec_from_file_location('navigation', path)
    navigation = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(navigation)
    server = ThreadingHTTPServer(('127.0.0.1', 0), Page)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    client = None
    try:
        with args.output.open('x') as out:
            client = navigation.Client(args.port, 120)
            client.call('WebDriver:NewSession', {'pageLoadStrategy': 'none'})
            client.call('WebDriver:SetTimeouts', {
                'script': 30000, 'pageLoad': 120000, 'implicit': 120000,
            })
            client.call('WebDriver:Navigate', {'url': f'http://127.0.0.1:{server.server_port}/'})
            for trial in range(args.rounds + 1):
                element = client.call('WebDriver:FindElement', {
                    'using': 'css selector', 'value': '#run',
                })['value']['element-6066-11e4-a52e-4f735466cecf']
                print(json.dumps({'trial_begin': trial, 'warmup': trial == 0}), flush=True)
                began = time.monotonic()
                client.call('WebDriver:ElementClick', {'id': element})
                done = 'done-' + str(trial + 1)
                client.call('WebDriver:FindElement', {
                    'using': 'css selector', 'value': '#' + done,
                })
                result = json.loads(client.script(
                    'return document.getElementById(' + json.dumps(done) + ').textContent;'))
                result.update(trial=trial, warmup=trial == 0,
                              host_ms=(time.monotonic() - began) * 1000)
                out.write(json.dumps(result) + '\n')
                out.flush()
                print(json.dumps(result), flush=True)
                if (not result.get('pixel_ok') or result.get('gl_error') != 0
                        or result.get('context_lost')):
                    raise RuntimeError('WebGL pixel/context check failed: ' + json.dumps(result))
    finally:
        if client is not None:
            try:
                client.call('WebDriver:DeleteSession')
            except (OSError, RuntimeError, EOFError):
                pass
            client.socket.close()
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
