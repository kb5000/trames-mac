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
import sys

spec = importlib.util.spec_from_file_location(
    'navigation', Path(__file__).with_name('benchmark-firefox-navigation.py'))
navigation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(navigation)


def measure(client, selector, seconds, warmup, check_input, check_volume=False):
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
      globalThis.tramesVideoBenchmark={{video:v,callbacks:0,ticket:null,events:[],listeners:[],gaps:[],lastFrame:null}};
      const s=globalThis.tramesVideoBenchmark;
      for(const event of ['playing','waiting','stalled','seeking','seeked','ended']){{
        const listener=()=>s.events.push({{event,wall:performance.now(),time:v.currentTime,ready:v.readyState}});
        v.addEventListener(event,listener);s.listeners.push([event,listener]);
      }}
      function frame(now,metadata){{if(s.lastFrame!==null)s.gaps.push({{wall:now,gap:now-s.lastFrame,media:metadata.mediaTime,decode_ms:metadata.processingDuration===undefined?null:metadata.processingDuration*1000}});s.lastFrame=now;s.callbacks++;s.ticket=v.requestVideoFrameCallback(frame);}}
      s.ticket=v.requestVideoFrameCallback(frame);
      return v.paused?v.play().then(()=>true):true;''')
    try:
        print("TRAMES_VIDEO_WARMUP", file=sys.stderr, flush=True)
        time.sleep(warmup)

        def snapshot():
            return client.script('''const s=globalThis.tramesVideoBenchmark,v=s.video,
              q=v.getVideoPlaybackQuality();return {wall:performance.now(),time:v.currentTime,
              size:[v.videoWidth,v.videoHeight],rate:v.playbackRate,ready:v.readyState,
              total:q.totalVideoFrames,dropped:q.droppedVideoFrames,callbacks:s.callbacks,
              paused:v.paused,error:v.error?.code??null,visibility:document.visibilityState,
              focus:document.hasFocus(),duration:v.duration,events:s.events,frame_gaps:s.gaps,
              buffered:Array.from({length:v.buffered.length},(_,i)=>[v.buffered.start(i),v.buffered.end(i)])};''')

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
                      dropped_frames=dropped,total_frames=frames,
                      playback_events=[event for event in after['events'] if event['wall'] >= before['wall']])
        def gaps_summary(events, start, end):
            gaps=sorted(item['gap'] for item in events if start <= item['wall'] <= end)
            if not gaps:
                return {'count':0}
            return {'count':len(gaps),'p50_ms':gaps[(len(gaps)-1)//2],
                    'p95_ms':gaps[min(len(gaps)-1,int(len(gaps)*.95))],
                    'p99_ms':gaps[min(len(gaps)-1,int(len(gaps)*.99))],
                    'max_ms':gaps[-1], 'over_50_ms':sum(g>50 for g in gaps),
                    'over_100_ms':sum(g>100 for g in gaps)}
        result['frame_gaps']=gaps_summary(after['frame_gaps'],before['wall'],after['wall'])
        if after['time'] >= before['time'] and not any(e['event']=='seeking' for e in result['playback_events']):
            result['media_speed']=(after['time']-before['time'])/elapsed
        if check_volume:
            volume_before=snapshot();changes=[]
            for level in (.05,.12,.03,.08):
                began=time.monotonic()
                client.script(f'document.querySelector({selector}).volume={level};return true;')
                changes.append({'volume':level,'setter_roundtrip_ms':(time.monotonic()-began)*1000})
                time.sleep(.5)
            volume_after=snapshot()
            result['volume_changes']=changes
            result['volume_frame_gaps']=gaps_summary(volume_after['frame_gaps'],volume_before['wall'],volume_after['wall'])
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
          s.video.cancelVideoFrameCallback(s.ticket);
          for(const [event,listener] of s.listeners)s.video.removeEventListener(event,listener);
          delete globalThis.tramesVideoBenchmark;return true;''')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port',type=int,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--selector',default='video')
    parser.add_argument('--url',help='Navigate the isolated test profile to this video page before measuring')
    parser.add_argument('--seconds',type=float,default=20)
    parser.add_argument('--warmup',type=float,default=5)
    parser.add_argument('--check-input',action='store_true')
    parser.add_argument('--check-volume',action='store_true',help='Test webpage volume changes after the FPS interval')
    args = parser.parse_args()
    if not 0 < args.port < 65536 or args.seconds <= 0 or args.warmup < 0:
        parser.error('positive port/seconds and non-negative warmup required')
    client = navigation.Client(args.port,30)
    try:
        client.call('WebDriver:NewSession',{'pageLoadStrategy':'none'})
        client.call('WebDriver:SetTimeouts',{'script':30000,'pageLoad':60000,'implicit':10000})
        if args.url:
            client.call('WebDriver:Navigate',{'url':args.url})
        result = measure(client,args.selector,args.seconds,args.warmup,args.check_input,args.check_volume)
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
