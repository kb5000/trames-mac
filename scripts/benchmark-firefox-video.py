#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure an HTML video in a separate localhost Marionette test profile.

Keep the window visible. This controller preserves browser graphics, decoder
and sandbox preferences. Frame callbacks count notifications; playback-quality
counters count accepted/dropped video frames, not physical scanout events.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import time

spec = importlib.util.spec_from_file_location(
    'navigation', Path(__file__).with_name('benchmark-firefox-navigation.py'))
navigation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(navigation)


def measure(client, selector, seconds, warmup, check_input):
    selector = json.dumps(selector)
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        state = client.script(f'''const v=document.querySelector({selector});
          return {{url:location.href,found:!!v,width:v?.videoWidth,error:v?.error?.code}};''')
        if state and state.get('width'):
            break
        if state and state.get('error'):
            raise RuntimeError(f'video error: {state}')
        time.sleep(.25)
    else:
        raise RuntimeError(f'video readiness timeout: {state}')
    client.script(f'''const v=document.querySelector({selector});
      globalThis.tramesVideoBenchmark={{video:v,callbacks:0,ticket:null}};
      const s=globalThis.tramesVideoBenchmark;
      function frame(){{s.callbacks++;s.ticket=v.requestVideoFrameCallback(frame);}}
      s.ticket=v.requestVideoFrameCallback(frame);
      return v.paused?v.play().then(()=>true):true;''')
    try:
        time.sleep(warmup)

        def snapshot():
            return client.script('''const s=globalThis.tramesVideoBenchmark,v=s.video,
              q=v.getVideoPlaybackQuality();return {wall:performance.now(),time:v.currentTime,
              size:[v.videoWidth,v.videoHeight],rate:v.playbackRate,ready:v.readyState,
              total:q.totalVideoFrames,dropped:q.droppedVideoFrames,callbacks:s.callbacks,
              paused:v.paused,error:v.error?.code??null,visibility:document.visibilityState};''')

        before = snapshot()
        start = time.monotonic()
        time.sleep(seconds)
        after = snapshot()
        elapsed = (after['wall'] - before['wall']) / 1000
        frames = after['total'] - before['total']
        dropped = after['dropped'] - before['dropped']
        if elapsed <= 0 or frames < 0 or dropped < 0:
            raise RuntimeError('video source/counters changed during measurement')
        result = dict(page=state['url'],elapsed_browser=elapsed,
                      elapsed_host=time.monotonic()-start,before=before,after=after,
                      accepted_fps=(frames-dropped)/elapsed,
                      callback_fps=(after['callbacks']-before['callbacks'])/elapsed,
                      dropped_frames=dropped,total_frames=frames)
        if check_input:
            client.script('''const d=document.createElement('div');d.id='trames-video-input-check';
              d.style='position:fixed;top:4px;left:4px;z-index:2147483647;background:white;color:black';
              d.innerHTML='<button id="trames-video-click">Click</button><input id="trames-video-text">';
              d.querySelector('button').onclick=()=>{d.dataset.clicked='yes';};document.body.append(d);
              return true;''')
            key = 'element-6066-11e4-a52e-4f735466cecf'
            button = client.call('WebDriver:FindElement', {
                'using':'css selector','value':'#trames-video-click'})['value'][key]
            field = client.call('WebDriver:FindElement', {
                'using':'css selector','value':'#trames-video-text'})['value'][key]
            start = time.monotonic()
            client.call('WebDriver:ElementClick', {'id':button})
            client.call('WebDriver:ElementSendKeys', {'id':field,'text':'Trames video'})
            result['interaction_ms'] = (time.monotonic()-start)*1000
            result['interaction'] = client.script('''const d=document.querySelector('#trames-video-input-check');
              const r={clicked:d.dataset.clicked,text:d.querySelector('input').value};d.remove();return r;''')
            if result['interaction'] != {'clicked':'yes','text':'Trames video'}:
                raise RuntimeError(f'input did not reach page: {result["interaction"]}')
        return result
    finally:
        client.script('''const s=globalThis.tramesVideoBenchmark;
          s.video.cancelVideoFrameCallback(s.ticket);delete globalThis.tramesVideoBenchmark;return true;''')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port',type=int,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--selector',default='video')
    parser.add_argument('--seconds',type=float,default=20)
    parser.add_argument('--warmup',type=float,default=5)
    parser.add_argument('--check-input',action='store_true')
    args = parser.parse_args()
    if not 0 < args.port < 65536 or args.seconds <= 0 or args.warmup < 0:
        parser.error('positive port/seconds and non-negative warmup required')
    client = navigation.Client(args.port,30)
    try:
        client.call('WebDriver:NewSession',{'pageLoadStrategy':'none'})
        client.call('WebDriver:SetTimeouts',{'script':30000,'pageLoad':60000,'implicit':10000})
        result = measure(client,args.selector,args.seconds,args.warmup,args.check_input)
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps({key:value for key,value in result.items() if key not in ('before','after')}))
    finally:
        try:
            client.call('WebDriver:DeleteSession')
        finally:
            client.socket.close()


if __name__ == '__main__':
    main()
