#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Disposable CLI configuration, PATH installation and custom ARM64 rootfs checks."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner', type=Path, required=True)
    p.add_argument('--installer', type=Path, required=True)
    p.add_argument('--alpine', type=Path, required=True)
    p.add_argument('--debian', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    runner = a.runner.resolve()
    records = []
    with tempfile.TemporaryDirectory(prefix='trames-cli-') as temp:
        base = Path(temp)
        env = dict(os.environ, TRAMES_HOME=str(base / 'registry'))

        def run(*args, ok=True, input=None):
            r = subprocess.run([str(runner), *args], env=env, input=input, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
            records.append(dict(args=args, code=r.returncode, stdout=r.stdout, stderr=r.stderr))
            (a.output / 'commands.json').write_text(json.dumps(records, indent=2) + '\n')
            assert (r.returncode == 0) == ok, records[-1]
            return r

        expected_version = run('--version').stdout.splitlines()[0]
        assert json.loads(run('config', '--global', 'show').stdout)['defaults']['distribution'] == 'alpine'
        run('config', '--global', 'set', 'defaults.cpus', '2', 'defaults.windows', 'false', 'defaults.network', 'off', 'default-environment', 'mine')
        # Invalid changes leave the last valid global file byte-for-byte intact.
        config_path = Path(run('config', '--global', 'path').stdout.strip())
        before = config_path.read_bytes()
        run('config', '--global', 'set', 'defaults.cpus', '0', ok=False)
        assert config_path.read_bytes() == before
        run('env', 'create', 'builtin', '--archive', str(a.alpine.resolve()))
        builtin = json.loads(run('env', 'config', 'builtin').stdout)
        assert builtin['cpus'] == 2 and not builtin['windows'] and builtin['command'] == ['/bin/sh', '-l']
        run('env', 'configure', 'builtin', '--', '/bin/sh', '-ec', 'echo CONFIGURED_COMMAND')
        assert 'CONFIGURED_COMMAND' in run('debug', 'env-run', 'builtin', '--no-pty').stdout
        assert 'OVERRIDE' in run('debug', 'env-run', 'builtin', '--no-pty', '/bin/echo', 'OVERRIDE').stdout
        run('config', '--global', 'set', 'default-environment', 'builtin')
        # Explicit env default reaches the same selected environment.
        assert run('config', '--global', 'get', 'default-environment').stdout.strip() == 'builtin'
        assert 'Usage: trames' in run().stdout
        run('config', '--global', 'set', 'defaults.cpus', '1')
        assert json.loads(run('env', 'config', 'builtin').stdout)['cpus'] == 2

        directory = base / 'my rootfs'
        (directory / 'usr/bin').mkdir(parents=True)
        (directory / 'lib').mkdir()
        with tarfile.open(a.alpine) as archive:
            for source, dest in [('./bin/busybox', 'usr/bin/busybox'), ('./lib/ld-musl-aarch64.so.1', 'lib/ld-musl-aarch64.so.1')]:
                target = directory / dest
                target.write_bytes(archive.extractfile(source).read())
                target.chmod(0o755)
        (directory / 'bin').symlink_to('usr/bin')
        (directory / 'usr/bin/sh').symlink_to('/bin/busybox')
        (directory / 'outside-link').symlink_to('/rootfs-host-sentinel')
        (directory / 'usr/bin/check').write_text('#!/bin/sh\necho SCRIPT_ENTRYPOINT\n')
        (directory / 'usr/bin/check').chmod(0o755)
        os.link(directory / 'usr/bin/check', directory / 'usr/bin/check-dir-hardlink')
        run('env', 'create', 'dir', '--rootfs', str(directory), '--', '/usr/bin/check')
        assert 'SCRIPT_ENTRYPOINT' in run('debug', 'env-run', 'dir', '--no-pty').stdout
        inodes = run('debug', 'env-run', 'dir', '--no-pty', '/bin/busybox', 'stat', '-c', '%i', '/usr/bin/check', '/usr/bin/check-dir-hardlink').stdout.splitlines()
        assert len(inodes) == 2 and inodes[0] == inodes[1], inodes
        assert 'usr/bin' in run('debug', 'env-run', 'dir', '--no-pty', '/bin/busybox', 'readlink', '/bin').stdout
        assert '/rootfs-host-sentinel' in run('debug', 'env-run', 'dir', '--no-pty', '/bin/busybox', 'readlink', '/outside-link').stdout
        # A flattened Debian rootfs with merged-/usr links, not a catalog pin.
        run('env', 'create', 'custom-debian', '--rootfs', str(a.debian.resolve()), '--sha256', hashlib.sha256(a.debian.read_bytes()).hexdigest())
        assert 'glibc' in run('debug', 'env-run', 'custom-debian', '--no-pty', '/usr/bin/getconf', 'GNU_LIBC_VERSION').stdout
        # Gzip archive and a >64 MiB file must work without the pinned importer's
        # old per-file limit or whole-expanded-image buffering.
        archive_path = base / 'rootfs.tar.gz'
        class Zeros:
            def read(self, n):
                return bytes(n)
        with tarfile.open(archive_path, 'w:gz', compresslevel=1) as archive:
            archive.add(directory, arcname='.', recursive=True)
            info = tarfile.TarInfo('opt/large-file')
            info.size = 70 * 1024 * 1024
            info.mode = 0o640
            archive.addfile(info, Zeros())
            info = tarfile.TarInfo('usr/bin/check-hardlink')
            info.type = tarfile.LNKTYPE
            info.linkname = 'usr/bin/check'
            info.mode = 0o755
            archive.addfile(info)
            info = tarfile.TarInfo('usr/bin/sh-hardlink')
            info.type = tarfile.LNKTYPE
            info.linkname = 'usr/bin/sh'
            archive.addfile(info)
            info = tarfile.TarInfo('dev/null')
            info.type = tarfile.CHRTYPE
            info.devmajor = 1
            info.devminor = 3
            info.mode = 0o666
            archive.addfile(info)
        run('env', 'create', 'gz', '--rootfs', str(archive_path))
        assert str(70 * 1024 * 1024) in run('debug', 'env-run', 'gz', '--no-pty', '/bin/busybox', 'stat', '-c', '%s', '/opt/large-file').stdout
        assert 'SCRIPT_ENTRYPOINT' in run('debug', 'env-run', 'gz', '--no-pty', '/usr/bin/check-hardlink').stdout
        assert '/bin/busybox' in run('debug', 'env-run', 'gz', '--no-pty', '/bin/busybox', 'readlink', '/usr/bin/sh-hardlink').stdout
        assert '666:1:3' in run('debug', 'env-run', 'gz', '--no-pty', '/bin/busybox', 'stat', '-c', '%a:%t:%T', '/dev/null').stdout
        plain = base / 'rootfs.tar'
        with gzip.open(archive_path, 'rb') as inp, plain.open('wb') as out:
            import shutil
            shutil.copyfileobj(inp, out)
        assert 'SCRIPT_ENTRYPOINT' in run('--rootfs', str(plain), '--storage', str(base / 'raw-store'), '--no-windows', '--no-pty', '/usr/bin/check').stdout
        run('env', 'create', 'digest-fail', '--rootfs', str(archive_path), '--sha256', '0' * 64, ok=False)
        assert not (base / 'registry/environments/digest-fail').exists()
        # Rejected archives never become visible environments or write a host path.
        for label, path in [('traversal', '../escaped'), ('absolute', str(base / 'escaped'))]:
            bad = base / f'{label}.tar'
            with tarfile.open(bad, 'w') as archive:
                info = tarfile.TarInfo(path)
                info.size = 1
                archive.addfile(info, io.BytesIO(b'x'))
            run('env', 'create', label, '--rootfs', str(bad), ok=False)
            assert not (base / 'registry/environments' / label).exists()
            assert not (base / 'escaped').exists()
        for label, link_type, target in [('symlink-loop', tarfile.SYMTYPE, 'bin'), ('hardlink-cycle', tarfile.LNKTYPE, 'bin/sh')]:
            bad = base / f'{label}.tar'
            with tarfile.open(bad, 'w') as archive:
                info = tarfile.TarInfo('bin' if link_type == tarfile.SYMTYPE else 'bin/sh')
                info.type = link_type
                info.linkname = target
                archive.addfile(info)
                if link_type == tarfile.SYMTYPE:
                    info = tarfile.TarInfo('bin/sh')
                    info.mode = 0o755
                    info.size = 1
                    archive.addfile(info, io.BytesIO(b'x'))
            run('env', 'create', label, '--rootfs', str(bad), ok=False)
            assert not (base / 'registry/environments' / label).exists()
        wrong = base / 'wrong'
        (wrong / 'bin').mkdir(parents=True)
        (wrong / 'bin/sh').write_bytes(b'\x7fELF\x02\x01' + bytes(12) + (62).to_bytes(2, 'little'))
        (wrong / 'bin/sh').chmod(0o755)
        run('env', 'create', 'x86', '--rootfs', str(wrong), ok=False)
        assert not (base / 'registry/environments/x86').exists()
        run('env', 'create', 'custom-systemd-missing', '--rootfs', str(directory), '--init', 'systemd', ok=False)
        backup = base / 'custom.backup.tar.gz'
        run('env', 'backup', 'dir', str(backup))
        run('env', 'restore', 'restored', str(backup))
        assert 'SCRIPT_ENTRYPOINT' in run('debug', 'env-run', 'restored', '--no-pty').stdout
        run('env', 'import', 'imported-custom', str(base / 'registry/environments/dir/store'))
        run('env', 'configure', 'imported-custom', '--', '/usr/bin/check')
        assert 'SCRIPT_ENTRYPOINT' in run('debug', 'env-run', 'imported-custom', '--no-pty').stdout

        # Exercise the actual installer in independent profile directories.
        # ZDOTDIR/XDG_CONFIG_HOME are supported shell settings; HOME is untouched.
        profiles = base / 'shell'
        profiles.mkdir()
        (profiles / '.zshrc').write_text('# user settings\nexport USER_SETTINGS_PRESERVED=yes\n')
        installed_bin = base / "bin with ' quote $dollar"
        install_env = dict(os.environ, SHELL='/bin/zsh', ZDOTDIR=str(profiles),
                           XDG_CONFIG_HOME=str(base / 'config'), TRAMES_APPLICATIONS_DIR=str(base / 'Applications'),
                           TRAMES_BIN_DIR=str(installed_bin))
        install_env['TRAMES_HOME'] = str(base / 'installer-registry')
        launchers = base / 'installer-registry/launchers'
        launchers.mkdir(parents=True)
        legacy = launchers / 'default.command'
        legacy.write_text('#!/bin/sh\nexport TRAMES_HOME=old\nexec trames-mac-run env start default\n')
        unrelated = launchers / 'personal.command'
        unrelated.write_text('#!/bin/sh\necho user launcher\n')
        app = runner.parents[2] if runner.parent.name == 'MacOS' else runner.parent / 'Trames.app'
        import plistlib
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        cli_name = 'trames-dev' if info['CFBundleIdentifier'].endswith('.dev') else 'trames'
        product_name = 'Trames Dev' if cli_name == 'trames-dev' else 'Trames'
        for _ in range(2):
            subprocess.run(['/bin/bash', str(a.installer.resolve()), str(app)], env=install_env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=30)
        assert ' -e default shell\n' in legacy.read_text()
        assert unrelated.read_text() == '#!/bin/sh\necho user launcher\n'
        for profile in ['.zprofile', '.zshrc']:
            text = (profiles / profile).read_text()
            assert text.count('# ' + product_name + ' CLI') == 1, text
        assert '# user settings' in (profiles / '.zshrc').read_text()
        query = subprocess.run(['/bin/zsh', '-lic', f'command -v {cli_name}; printf "%s\\n" "$USER_SETTINGS_PRESERVED"; {cli_name} --version'],
                               env=install_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=20)
        assert query.returncode == 0 and str(installed_bin / cli_name) in query.stdout and 'yes' in query.stdout and expected_version in query.stdout, query
        # An interactive non-login shell gets PATH too, with no duplication.
        query = subprocess.run(['/bin/zsh', '-ic', 'print -r -- $PATH'], env=install_env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=True)
        assert query.stdout.count(str(installed_bin)) == 1, query.stdout
    (a.output / 'result.json').write_text(json.dumps(dict(checks=len(records), runner_sha256=hashlib.sha256(runner.read_bytes()).hexdigest(), passed=True), indent=2) + '\n')
    print('PASS: global/per-environment config, generic rootfs, entrypoints, rollback and idempotent PATH installation')


if __name__ == '__main__':
    main()
