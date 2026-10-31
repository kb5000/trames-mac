#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build a versioned Mac distribution with checksums and paired source exports.

A distributable release requires Developer ID signing. --development permits an
explicitly labelled local ad-hoc package. Notarization uses a caller-selected
notarytool keychain profile; this script never reads credentials itself.
"""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import zipfile


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def main():
    root = Path(__file__).resolve().parents[1]
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--app', type=Path, help='Application bundle; defaults to the matching stable/dev build')
    p.add_argument('--output', type=Path, default=root / 'target/dist')
    p.add_argument('--unnotarized', '--development', dest='development', action='store_true', help='Local ad-hoc package; independent of stable/dev product channel')
    p.add_argument('--notarize-profile')
    p.add_argument('--corresponding-source', type=Path, action='append', default=[], help='Required for a release: original static-driver sources, patches and build recipes')
    args = p.parse_args()
    default = root / 'target/release' / ('Trames Dev.app' if args.development and (root / 'target/release/Trames Dev.app').is_dir() else 'Trames.app')
    app = (args.app or default).resolve()
    run('/usr/bin/codesign', '--verify', '--strict', str(app))
    signature = subprocess.check_output(['/usr/bin/codesign', '-dv', '--verbose=4', str(app)], stderr=subprocess.STDOUT, text=True)
    if not args.development and ('Authority=Developer ID Application:' not in signature or 'runtime' not in signature):
        p.error('Release requires a hardened Developer ID Application signature; use --unnotarized for a local test package')
    if not args.development and (not args.notarize_profile or not args.corresponding_source):
        p.error('Release requires --notarize-profile and --corresponding-source for the native driver/dependencies')
    if args.development and args.notarize_profile:
        p.error('Development archives are not submitted for notarization')
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    version = info['CFBundleShortVersionString']
    channel = info.get('TramesChannel', 'stable')
    if channel not in ('stable', 'dev'):
        p.error('Unknown application channel')
    product_name = 'Trames' if channel == 'stable' else 'Trames Dev'
    if info['CFBundleIdentifier'] != 'org.trames.Trames.mac' + ('' if channel == 'stable' else '.dev'):
        p.error('Bundle identity does not match the product channel')
    if not args.development and channel != 'stable':
        p.error('A production release requires the stable product channel')
    revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
    runtime = subprocess.check_output(['git', '-C', str(root / 'runtime'), 'rev-parse', 'HEAD'], text=True).strip()
    pin = json.loads((root / 'runtime-version.json').read_text())
    if runtime != pin['revision'] and not args.development:
        p.error('Runtime source does not match the pinned revision')
    for repo in (root, root / 'runtime'):
        if not args.development and subprocess.check_output(['git', '-C', str(repo), 'status', '--porcelain']).strip():
            p.error(f'Release source is dirty: {repo}')
    args.output.mkdir(parents=True, exist_ok=True)
    basename = f"{'Trames' if channel == 'stable' else 'Trames-Dev'}-{version}-mac-arm64" + ('-unnotarized' if args.development else '')
    output = args.output / f'{basename}.zip'
    if output.exists():
        p.error('Output already exists; choose a new output directory')
    with tempfile.TemporaryDirectory(prefix='trames-package-') as tmp:
        stage = Path(tmp) / basename
        stage.mkdir()
        run('/usr/bin/ditto', str(app), str(stage / (product_name + '.app')))
        shutil.copy2(root / 'scripts/install-mac.sh', stage / 'install.command')
        shutil.copy2(root / 'LICENSE', stage / 'LICENSE')
        shutil.copy2(root / 'docs/guides/environments.md', stage / 'README.md')
        sources = stage / 'Sources'
        sources.mkdir()
        for repo, name in ((root, 'trames-mac'), (root / 'runtime', 'trames-runtime')):
            run('git', '-C', str(repo), 'archive', '--format=tar.gz', f'--output={sources / (name + ".tar.gz")}', 'HEAD')
        for source in args.corresponding_source:
            if not source.is_file():
                p.error(f'Missing source archive: {source}')
            shutil.copy2(source, sources / source.name)
        (stage / 'BUILD.json').write_text(json.dumps({'version': version, 'product_revision': revision, 'runtime_revision': runtime, 'development': args.development, 'channel': channel, 'notarized': not args.development, 'architecture': 'arm64'}, indent=2) + '\n')
        if args.notarize_profile:
            submission = Path(tmp) / 'notarize.zip'
            run('/usr/bin/ditto', '-c', '-k', '--keepParent', str(stage / (product_name + '.app')), str(submission))
            run('xcrun', 'notarytool', 'submit', str(submission), '--keychain-profile', args.notarize_profile, '--wait')
            run('xcrun', 'stapler', 'staple', str(stage / (product_name + '.app')))
            run('xcrun', 'stapler', 'validate', str(stage / (product_name + '.app')))
            run('/usr/sbin/spctl', '--assess', '--type', 'execute', str(stage / (product_name + '.app')))
        inventory = {}
        for path in sorted(stage.rglob('*')):
            if path.is_file():
                h = hashlib.sha256()
                with path.open('rb') as f:
                    for block in iter(lambda: f.read(1024 * 1024), b''):
                        h.update(block)
                inventory[str(path.relative_to(stage))] = h.hexdigest()
        (stage / 'SHA256SUMS.json').write_text(json.dumps(inventory, indent=2) + '\n')
        run('/usr/bin/ditto', '-c', '-k', '--keepParent', str(stage), str(output))
    with zipfile.ZipFile(output) as archive:
        bad = archive.testzip()
        if bad:
            raise RuntimeError(f'Corrupt package member: {bad}')
    h = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix('.zip.sha256').write_text(f'{h}  {output.name}\n')
    print(output)


if __name__ == '__main__':
    main()
