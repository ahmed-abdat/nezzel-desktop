#!/usr/bin/env python3
"""Prepare a pinned gallery-dl sidecar; source builds run natively, never cross-compile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = json.loads((ROOT / 'scripts/gallery-dl-manifest.json').read_text())
STUB = b'NEZZEL_GALLERY_UNAVAILABLE\n'


def checked_download(asset, destination):
    with urllib.request.urlopen(asset['url'], timeout=60) as response:
        payload = response.read(128 * 1024 * 1024 + 1)
    if len(payload) > 128 * 1024 * 1024 or hashlib.sha256(payload).hexdigest() != asset['sha256']:
        raise ValueError('gallery-dl checksum mismatch')
    destination.write_bytes(payload)


def host_target():
    machine = platform.machine().lower()
    arch = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64', 'amd64': 'x86_64'}[machine]
    suffix = {'darwin': 'apple-darwin', 'linux': 'unknown-linux-gnu', 'win32': 'pc-windows-msvc'}[sys.platform]
    return f'{arch}-{suffix}'


def smoke(binary):
    command = [str(binary), '--config-ignore']
    if subprocess.check_output([*command, '--version'], text=True, timeout=30).strip() != MANIFEST['version']:
        raise ValueError('gallery-dl executable version mismatch')
    # This iterates the real registry and imports every extractor. --version
    # alone succeeds even when PyInstaller omitted dynamically loaded modules.
    output = subprocess.check_output([*command, '--list-extractors'], text=True, timeout=30)
    categories = {line.split()[1] for line in output.splitlines() if line.startswith('Category: ')}
    if not {'twitter', 'instagram', 'reddit', 'bluesky', 'pinterest'} <= categories:
        raise ValueError('gallery-dl executable is missing required extractors')
    # DirectlinkExtractor.items() resolves this reserved-domain URL locally.
    # Simulation instantiates a real extraction job without DNS/HTTP or files.
    with tempfile.TemporaryDirectory(prefix='nezzel-gallery-smoke-') as tmp:
        output = subprocess.check_output([*command, '--no-colors', '-s', '--directory', tmp, '--filename', 'sidecar-smoke.{extension}', '--', 'https://nezzel.invalid/sidecar-smoke.jpg'], text=True, timeout=30)
        if 'sidecar-smoke.jpg' not in output or any(Path(tmp).iterdir()):
            raise ValueError('gallery-dl offline extraction smoke failed')


def install(target, allow_unavailable=False, source_archive=None):
    directory = ROOT / 'src-tauri/binaries'
    directory.mkdir(parents=True, exist_ok=True)
    out = directory / ('gallery-dl-' + target + ('.exe' if 'windows' in target else ''))
    metadata = out.with_name(out.name + '.json')
    # macOS: a onefile build extracts Python.framework to $TMPDIR at every start, and the
    # bundle's hardened-runtime signature then refuses to load it (library validation).
    # Ship a PyInstaller onedir as a resource instead, like yt-dlp: tools/gallery-dl/gallery-dl.
    onedir = directory / 'tools/gallery-dl' if 'apple-darwin' in target else None
    entry = onedir / 'gallery-dl' if onedir else out
    lock = ROOT / 'scripts/gallery-dl-build-requirements.txt'
    recipe = json.dumps(MANIFEST, sort_keys=True).encode() + lock.read_bytes() + Path(__file__).read_bytes()
    identity = hashlib.sha256(target.encode() + recipe).hexdigest()
    try:
        variant = MANIFEST['targets'].get(target)
        if variant is None:
            raise ValueError(f'Unsupported gallery-dl target: {target}')
        expected = MANIFEST['assets'][variant]['sha256'] if variant != 'native' else None
        cached = json.loads(metadata.read_text()) if metadata.is_file() else {}
        if entry.is_file() and entry.stat().st_size > len(STUB):
            actual = hashlib.sha256(entry.read_bytes()).hexdigest()
            if actual == expected or (variant == 'native' and cached.get('identity') == identity and cached.get('sha256') == actual):
                if target == host_target():
                    smoke(entry)
                (directory / 'gallery-dl.sha256').write_text(actual + '\n')
                return entry
        if variant == 'native' and target != host_target():
            raise ValueError('gallery-dl must be built on a native target runner')
        if variant == 'native' and not os.environ.get('CI') and not os.environ.get('HEAVY_HELD'):
            # The global heavy wrapper is reentrant; never create a private lock to bypass it.
            command = ['heavy', '-n', 'nezzel gallery-dl', '--', sys.executable, str(Path(__file__).resolve()), '--target', target]
            if source_archive:
                command += ['--source-archive', str(source_archive)]
            subprocess.run(command, check=True)
            return entry
        with tempfile.TemporaryDirectory(prefix='nezzel-gallery-build-') as tmp:
            work = Path(tmp)
            binary = work / 'gallery-dl'
            if variant != 'native':
                checked_download(MANIFEST['assets'][variant], binary)
            else:
                python = shutil.which('python3.14') or sys.executable
                version = subprocess.check_output([python, '-c', 'import platform; print(platform.python_version())'], text=True).strip()
                if version != MANIFEST['python']:
                    raise ValueError(f"gallery-dl build requires Python {MANIFEST['python']}, got {version}")
                archive = work / 'source.tar.gz'
                if source_archive:
                    checked_download({'url': Path(source_archive).resolve().as_uri(), 'sha256': MANIFEST['source']['sha256']}, archive)
                else:
                    checked_download(MANIFEST['source'], archive)
                with tarfile.open(archive) as tar:
                    tar.extractall(work, filter='data')
                source = work / ('gallery_dl-' + MANIFEST['version'])
                environment = work / 'venv'
                subprocess.run([python, '-m', 'venv', str(environment)], check=True)
                interpreter = str(environment / 'bin/python')
                subprocess.run([interpreter, '-m', 'pip', '--disable-pip-version-check', '--no-cache-dir', 'install', '--require-hashes', '--only-binary=:all:', '-r', str(lock)], check=True)
                subprocess.run([interpreter, '-m', 'PyInstaller', '--noconfirm', '--onedir' if onedir else '--onefile', '--console', '--name', 'gallery-dl', '--paths', str(source), '--collect-submodules', 'gallery_dl', '--additional-hooks-dir', str(source / 'scripts'), '--exclude-module', 'pkg_resources', '--distpath', str(work / 'dist'), '--workpath', str(work / 'build'), '--specpath', str(work), str(source / 'gallery_dl/__main__.py')], cwd=source, check=True)
                if onedir:
                    binary = work / 'dist/gallery-dl/gallery-dl'
                else:
                    shutil.copyfile(work / 'dist/gallery-dl', binary)
            binary.chmod(0o755)
            if target == host_target():
                smoke(binary)
            digest = hashlib.sha256(binary.read_bytes()).hexdigest()
            if onedir:
                install_onedir(binary.parent, onedir)
            else:
                temporary = out.with_name(out.name + '.tmp')
                shutil.copyfile(binary, temporary)
                temporary.chmod(0o755)
                temporary.replace(out)
            metadata.write_text(json.dumps({'identity': identity, 'sha256': digest}) + '\n')
            (directory / 'gallery-dl.sha256').write_text(digest + '\n')
            print(f'{entry.name}: {digest}')
            return entry
    except Exception:
        if not allow_unavailable:
            raise
        if onedir:
            if onedir.exists():
                shutil.rmtree(onedir)
            onedir.mkdir(parents=True)
        entry.write_bytes(STUB)
        entry.chmod(0o755)
        (directory / 'gallery-dl.sha256').write_text(hashlib.sha256(STUB).hexdigest() + '\n')
        print('warning: gallery-dl unavailable for this development/test build', file=sys.stderr)
        return entry


def install_onedir(source, destination):
    # Replace atomically enough for a build tree: copy beside, then swap.
    staging = destination.with_name(destination.name + '.tmp')
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(source, staging, symlinks=True)
    if destination.exists():
        shutil.rmtree(destination)
    staging.rename(destination)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', default=os.environ.get('TAURI_ENV_TARGET_TRIPLE') or host_target())
    parser.add_argument('--allow-unavailable', action='store_true')
    parser.add_argument('--source-archive', type=Path)
    args = parser.parse_args()
    install(args.target, args.allow_unavailable, args.source_archive)


if __name__ == '__main__':
    main()
