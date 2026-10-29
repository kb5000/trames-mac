#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure real link navigation in an isolated localhost Marionette profile.

Keep the Trames window visible. This controller does not change graphics,
sandbox, cache, or process preferences. First visits and warm rounds are labelled
separately. Content markers indicate DOM readiness, not screen presentation.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import threading
import time
import uuid
from urllib.parse import parse_qs, urlsplit

CASES = {
    'wiki': ('Wikipedia — Firefox article', 'https://en.wikipedia.org/wiki/Firefox',
             ('xpath', "//*[@id='mw-content-text']//p[contains(.,'Mozilla Firefox')]")),
    'github': ('GitHub — Firefox source', 'https://github.com/mozilla-firefox/firefox',
               ('css selector', 'main a[href*="/mozilla-firefox/firefox/tree/"]')),
    'bilibili': ('Bilibili home', 'https://www.bilibili.com/',
                 ('xpath', "(//main//h3/a[contains(@href,'/video/')])[4]")),
}


class Client:
    def __init__(self, port, timeout):
        self.socket = socket.create_connection(('127.0.0.1', port), timeout=10)
        self.socket.settimeout(timeout + 20)
        self.counter = 0
        self.receive()

    def receive(self):
        size = bytearray()
        while True:
            char = self.socket.recv(1)
            if not char:
                raise EOFError('Firefox closed the automation connection')
            if char == b':':
                break
            size.extend(char)
        remaining = int(size)
        body = bytearray()
        while remaining:
            block = self.socket.recv(remaining)
            if not block:
                raise EOFError('Firefox closed an incomplete response')
            body.extend(block)
            remaining -= len(block)
        return json.loads(body)

    def call(self, name, parameters=None):
        self.counter += 1
        body = json.dumps([0, self.counter, name, parameters or {}], separators=(',', ':')).encode()
        self.socket.sendall(str(len(body)).encode() + b':' + body)
        response = self.receive()
        if response[:2] != [1, self.counter]:
            raise RuntimeError('automation response did not match the pending command')
        if response[2] is not None:
            raise RuntimeError(str(response[2]))
        return response[3]

    def script(self, source):
        result = self.call('WebDriver:ExecuteScript', {
            'script': source, 'args': [], 'newSandbox': False,
            'sandbox': 'default', 'line': 0, 'filename': 'navigation-benchmark',
        })
        # Navigation can replace the script's document before its result arrives.
        # A transient empty result is neither content readiness nor an RPC failure.
        return result.get('value') if isinstance(result, dict) else None


class Launcher(BaseHTTPRequestHandler):
    def do_GET(self):
        html = '<!doctype html><meta charset="utf-8"><title>Navigation benchmark launcher</title>'
        html += '<style>body{font:24px sans-serif;margin:32px}a{display:block;padding:18px;margin:16px 0}</style>'
        token = parse_qs(urlsplit(self.path).query).get('run', [''])[0]
        token = token if len(token) == 32 and all(c in '0123456789abcdef' for c in token) else 'default'
        html += f'<h1 id="launcher-{token}">Navigation benchmark launcher</h1>'
        html += ''.join(f'<a id="{key}" href="{url}">{label}</a>' for key, (label, url, _) in CASES.items())
        data = html.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(data)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_args):
        pass


def measure(client, launcher, case, trial, timeout):
    token = uuid.uuid4().hex
    client.call('WebDriver:Navigate', {'url': launcher + '?run=' + token})
    # A new unique marker prevents accepting an old launcher document while the
    # requested navigation is still pending. Browser-owned implicit waits avoid
    # repeatedly compiling injected scripts during expensive page parsing.
    client.call('WebDriver:FindElement', {'using': 'css selector', 'value': '#launcher-' + token})
    element = client.call('WebDriver:FindElement', {'using': 'css selector', 'value': '#' + case})['value']
    element_id = element['element-6066-11e4-a52e-4f735466cecf']
    start = time.monotonic()
    result = {'case': case, 'trial': trial, 'click_start_unix': time.time()}
    client.call('WebDriver:ElementClick', {'id': element_id})
    result['click_command_ms'] = (time.monotonic() - start) * 1000
    using, selector = CASES[case][2]
    try:
        client.call('WebDriver:FindElement', {'using': using, 'value': selector})
        result['content_ms'] = (time.monotonic() - start) * 1000
    except RuntimeError as error:
        if 'no such element' not in str(error):
            raise
        result['content_timeout_seconds'] = timeout
        result['failure'] = client.script('return {title:document.title,uri:document.documentURI,text:document.body.innerText.slice(0,600)};')
    result['observed_elapsed_ms'] = (time.monotonic() - start) * 1000
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port', type=int, required=True, help='Loopback Marionette port of a separate test profile')
    parser.add_argument('--output', type=Path, required=True, help='Local JSONL results; records are appended')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--warmups', type=int, default=1)
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--cases', nargs='+', choices=list(CASES), default=list(CASES))
    args = parser.parse_args()
    if not 0 < args.port < 65536 or args.rounds < 1 or args.warmups < 0 or args.timeout <= 0:
        parser.error('positive port, rounds, timeout and non-negative warmups required')
    server = ThreadingHTTPServer(('127.0.0.1', 0), Launcher)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    launcher = f'http://127.0.0.1:{server.server_port}/launch'
    client = Client(args.port, args.timeout)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        client.call('WebDriver:NewSession', {'pageLoadStrategy': 'none'})
        client.call('WebDriver:SetTimeouts', {'script': 10000, 'pageLoad': int(args.timeout * 1000), 'implicit': int(args.timeout * 1000)})
        with args.output.open('a') as output:
            for trial in range(args.warmups + args.rounds):
                for case in args.cases:
                    try:
                        result = measure(client, launcher, case, trial, args.timeout)
                    except (OSError, EOFError, RuntimeError) as error:
                        result = {'case': case, 'trial': trial, 'automation_error': str(error)}
                        output.write(json.dumps(result) + '\n')
                        output.flush()
                        raise
                    result['warmup'] = trial < args.warmups
                    output.write(json.dumps(result) + '\n')
                    output.flush()
                    print(json.dumps({'case': case, 'trial': trial,
                                      'warmup': result['warmup'], 'content_ms': result.get('content_ms'),
                                      'timeout': result.get('content_timeout_seconds')}), flush=True)
    finally:
        try:
            client.call('WebDriver:DeleteSession')
        except (OSError, EOFError, RuntimeError):
            pass
        client.socket.close()
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
