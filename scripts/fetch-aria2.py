#!/usr/bin/env python3
"""Prepare a Tauri sidecar from hash-pinned official aria2 1.37.0 sources/assets.
Run native macOS/Linux builds under the machine's heavy wrapper. No package installs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
STUB_MARKER = b'NEZZEL_ARIA2_UNAVAILABLE\n'
MANIFEST = json.loads((ROOT / 'scripts/aria2-manifest.json').read_text())


def checked_download(asset, destination):
    with urllib.request.urlopen(asset['url'], timeout=120) as response:
        destination.write_bytes(response.read())
    actual = hashlib.sha256(destination.read_bytes()).hexdigest()
    if actual != asset['sha256']:
        raise RuntimeError(f'aria2 checksum mismatch: {actual}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', default=os.environ.get('TAURI_ENV_TARGET_TRIPLE'))
    parser.add_argument('--source-archive', type=Path)
    args = parser.parse_args()
    host = next(line.split(': ', 1)[1] for line in subprocess.check_output(['rustc', '-vV'], text=True).splitlines() if line.startswith('host:'))
    target = args.target or host
    out = ROOT / 'src-tauri/binaries' / ('aria2c-' + target + ('.exe' if 'windows' in target else ''))
    checksum = out.with_suffix(out.suffix + '.sha256')
    out.parent.mkdir(parents=True, exist_ok=True)
    try:
        supported = MANIFEST['targets']
        if target not in supported:
            raise RuntimeError(f'aria2 sidecar unavailable for {target}; see scripts/aria2-manifest.json')
        if out.is_file() and not out.read_bytes().startswith(STUB_MARKER) and checksum.is_file() and checksum.read_text().split() and hashlib.sha256(out.read_bytes()).hexdigest() == checksum.read_text().split()[0]:
            return
        with tempfile.TemporaryDirectory(prefix='nezzel-aria2-') as tmp:
            work = Path(tmp)
            if supported[target] == 'official-win64':
                archive = work / 'aria2.zip'
                checked_download(MANIFEST['win64'], archive)
                with zipfile.ZipFile(archive) as zipped:
                    member = next(n for n in zipped.namelist() if n.endswith('/aria2c.exe'))
                    out.write_bytes(zipped.read(member))
            else:
                if target != host:
                    raise RuntimeError('Build aria2 natively on the target runner (no implicit cross compiler)')
                archive = work / 'aria2.tar.xz'
                if args.source_archive:
                    shutil.copyfile(args.source_archive, archive)
                    if hashlib.sha256(archive.read_bytes()).hexdigest() != MANIFEST['source']['sha256']:
                        raise RuntimeError('aria2 source checksum mismatch')
                else:
                    checked_download(MANIFEST['source'], archive)
                with tarfile.open(archive) as tar:
                    tar.extractall(work, filter='data')
                source = work / ('aria2-' + MANIFEST['version'])
                flags = ['--disable-nls', '--disable-bittorrent', '--disable-metalink', '--disable-websocket', '--disable-shared', '--without-libssh2', '--without-libxml2', '--without-sqlite3', '--without-libgmp', '--without-libnettle', '--without-libgcrypt', '--without-libcares', '--without-gnutls']
                env = os.environ.copy()
                if 'apple-darwin' in target:
                    # Apple TLS + system libz/libc++: no Homebrew or copied dylib dependencies.
                    flags += ['--with-appletls', '--without-openssl']
                    env['MACOSX_DEPLOYMENT_TARGET'] = '11.0'
                else:
                    # Requires static OpenSSL, zlib and libc development archives on the native runner.
                    flags += ['--with-openssl', '--without-appletls']
                    env['ARIA2_STATIC'] = 'yes'
                subprocess.run(['./configure', *flags], cwd=source, env=env, check=True)
                subprocess.run(['make', '-j2'], cwd=source, env=env, check=True)
                shutil.copyfile(source / 'src/aria2c', out)
            out.chmod(0o755)
            digest = hashlib.sha256(out.read_bytes()).hexdigest()
            checksum.write_text(f'{digest}  {out.name}\n')
            # Verify HTTPS support and record dynamic dependencies during native build QA.
            if target == host:
                version = subprocess.check_output([str(out), '--version'], text=True)
                if 'HTTPS' not in version:
                    out.unlink()
                    raise RuntimeError('aria2 build has no HTTPS support')
                print(version.splitlines()[0])
            print(f'{out.name}: {digest}')
    except Exception as error:
        # Optional acceleration must never block the application build. The runtime
        # recognizes this marker before treating a sidecar as available/executable.
        out.write_bytes(STUB_MARKER)
        out.chmod(0o755)
        checksum.write_text(f'{hashlib.sha256(STUB_MARKER).hexdigest()}  {out.name}\n')
        print(f'warning: aria2 disabled for {target}: {error}', file=sys.stderr)


if __name__ == '__main__':
    main()
