#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable real-HVF acceptance of scoped host controls and streamed output."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--alpine', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    rows = []
    with tempfile.TemporaryDirectory(prefix='trames-controls-') as temporary:
        env = dict(os.environ, TRAMES_HOME=temporary)
        runner = str(args.runner.resolve())

        def run(*argv, code=0):
            result = subprocess.run([runner, *argv], env=env, capture_output=True,
                                    text=True, timeout=30)
            rows.append(dict(argv=argv, code=result.returncode,
                             stdout=result.stdout, stderr=result.stderr))
            (args.output / 'commands.json').write_text(json.dumps(rows, indent=2) + '\n')
            assert result.returncode == code, (argv, result.returncode, len(result.stdout), result.stderr[-5000:])
            return result.stdout

        def guest(*argv, **kwargs):
            return run('debug', 'env-run', 'lab', '--no-windows', '--no-pty', *argv, **kwargs)

        run('env', 'create', 'lab', '--archive', str(args.alpine.resolve()),
            '--cpus', '2', '--vulkan', 'false', '--network', 'none')
        guest('/bin/sh', '-ec', 'test -e /dev/trames/info; test -e /dev/trames/clipboard; '
              'test ! -e /dev/trames/host-control; test ! -e /dev/trames/host-exec')
        run('-e', 'lab', 'config', 'set', 'interop.clipboard.enabled', 'false')
        guest('/bin/sh', '-ec', 'test ! -e /dev/trames/clipboard')
        run('env', 'grant', 'lab', 'host-exec', 'on')
        guest('/bin/sh', '-ec', 'test ! -e /dev/trames/host-exec')
        run('-e', 'lab', 'config', 'set', 'interop.host_exec.enabled', 'true')
        output = guest('/usr/local/bin/trames', 'host-exec', '--', '/usr/bin/uname', '-s')
        assert 'Darwin' in output
        # Larger than both pipe and broker buffers: each UTF-8 character must
        # survive arbitrary pipe read boundaries and guest backpressure.
        code = 'import sys; sys.stdout.write("中文🧪"*12000); sys.stderr.write("end\\n"); sys.exit(7)'
        output = guest('/usr/local/bin/trames', 'host-exec', '--', '/usr/bin/python3', '-c', code, code=7)
        frames = [json.loads(line)['result'] for line in output.splitlines()]
        assert ''.join(frame.get('stdout', '') for frame in frames) == '中文🧪' * 12000
        assert ''.join(frame.get('stderr', '') for frame in frames) == 'end\n'
        assert frames[-1] == dict(done=True, exit_code=7)
        quoted = 'space quote" slash\\ tab\t line\nend'
        output = guest('/usr/local/bin/trames', 'host-exec', '--', '/usr/bin/printf', '%s', quoted)
        assert ''.join(json.loads(line)['result'].get('stdout', '') for line in output.splitlines()) == quoted
        run('env', 'grant', 'lab', 'host-cli', 'on')
        run('-e', 'lab', 'config', 'set', 'interop.host_cli.enabled', 'true')
        assert 'Trames' in guest('/usr/local/bin/trames', 'host-cli', '--', '--version')
        denied = guest('/usr/local/bin/trames', 'host-cli', '--', 'env', 'grant', 'lab', 'host-exec', 'on', code=1)
        assert 'host_error' in denied
        run('env', 'grant', 'lab', 'host-exec', 'off')
        guest('/bin/sh', '-ec', 'test ! -e /dev/trames/host-exec; test -e /dev/trames/host-control; test ! -e /dev/trames/clipboard')
    (args.output / 'result.json').write_text(json.dumps(dict(passed=True,
        scoped_devices=True, independent_grants=True, utf8_stream_bytes=120000,
        exit_status=True, delegation_restrictions=True), indent=2) + '\n')
    print('PASS: scoped devices, independent grants, Unicode/backpressure, exit status and safe CLI delegation')


if __name__ == '__main__':
    main()
