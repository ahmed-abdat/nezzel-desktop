#!/usr/bin/env python3
"""Prepare installer tools from the runtime manifest; no release discovery or new URLs."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
import tarfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = json.loads((ROOT / 'src-tauri/src/binaries/manifest.json').read_text())
TOOLS = ('yt-dlp', 'ffmpeg', 'ffprobe', 'deno')
# Exact pinned binaries checked with -L: both report --enable-nonfree and
# "not legally redistributable". A different manifest asset is required.
NONREDISTRIBUTABLE_SHA256 = {
    'a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584',
    'bb2db6f5d8cef919da12fbf592119a987202a8c060a886f3cab091f9cab90b64',
}
TARGETS = {f'{arch}-{suffix}': f'{os_name}-{arch}'
           for arch in ('x86_64', 'aarch64')
           for suffix, os_name in (('apple-darwin', 'darwin'), ('unknown-linux-gnu', 'linux'), ('pc-windows-msvc', 'windows'))}


def host_target():
    arch = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64', 'amd64': 'x86_64'}[platform.machine().lower()]
    suffix = {'darwin': 'apple-darwin', 'linux': 'unknown-linux-gnu', 'win32': 'pc-windows-msvc'}[sys.platform]
    return f'{arch}-{suffix}'


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def checked_download(asset, destination):
    # Some mirrors (ffmpeg.martin-riedl.de) refuse Python's default User-Agent.
    request = urllib.request.Request(asset['url'], headers={'User-Agent': 'Nezzel-build/1.0 (+https://github.com/ahmed-abdat/nezzel-desktop)'})
    with urllib.request.urlopen(request, timeout=120) as response, destination.open('wb') as output:
        shutil.copyfileobj(response, output)
    if sha256(destination) != asset['sha256']:
        destination.unlink()
        raise ValueError(f"checksum mismatch: {asset['url']}")


def safe_path(root, name):
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or '\\' in name or ':' in name:
        raise ValueError(f'unsafe archive path: {name}')
    return root.joinpath(*path.parts)


def unpack(archive, asset, work, name):
    zipped = zipfile.is_zipfile(archive)
    if not zipped and not asset.get('entry') and not asset.get('bundle'):
        return archive
    extracted = work / 'extracted'
    extracted.mkdir()
    # Preserve the onedir Python runtime, including its original relative layout.
    if zipped:
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                dest = safe_path(extracted, member.filename)
                mode = member.external_attr >> 16
                if stat.S_ISLNK(mode):
                    raise ValueError(f'unsupported archive symlink: {member.filename}')
                if member.is_dir():
                    dest.mkdir(parents=True, exist_ok=True)
                else:
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.open(member) as source, dest.open('wb') as output:
                        shutil.copyfileobj(source, output)
                    dest.chmod((mode & 0o777) or 0o644)
    else:
        with tarfile.open(archive, 'r:*') as bundle:
            for member in bundle:
                dest = safe_path(extracted, member.name)
                if member.isdir():
                    dest.mkdir(parents=True, exist_ok=True)
                elif member.isfile():
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.extractfile(member) as source, dest.open('wb') as output:
                        shutil.copyfileobj(source, output)
                    dest.chmod((member.mode & 0o777) or 0o644)
                else:
                    raise ValueError(f'unsupported archive entry: {member.name}')
    rules = asset.get('bundle')
    if rules and rules.get('keep_folder'):
        folder = safe_path(extracted, rules['folder_name']) if rules.get('folder_name') else extracted
        # Some release ZIPs put the onedir contents directly at the archive root.
        if not folder.is_dir():
            folder = extracted
        entry = safe_path(folder, rules['entry'])
        renamed = safe_path(folder, rules.get('rename_entry_to', rules['entry']))
        if not entry.is_file():
            raise ValueError(f'missing {name} bundle entry: {entry}')
        if entry != renamed:
            entry.rename(renamed)
        renamed.chmod(0o755)
        return folder
    entry = safe_path(extracted, asset['entry'])
    if not entry.is_file():
        raise ValueError(f'missing {name} archive entry: {entry}')
    return entry


def install(target):
    if target not in TARGETS:
        raise ValueError(f'unsupported tool target: {target}')
    blocked = [name for name in TOOLS
               if MANIFEST['tools'][name]['files'][TARGETS[target]]['sha256'] in NONREDISTRIBUTABLE_SHA256]
    if blocked:
        raise ValueError(f"manifest pins nonfree, nonredistributable binaries for {target}: {', '.join(blocked)}; replace those manifest assets before bundling")
    binaries = ROOT / 'src-tauri/binaries'
    resources = binaries / 'tools'
    resources.mkdir(parents=True, exist_ok=True)
    index_path = resources / 'index.json'
    try:
        cached = json.loads(index_path.read_text())
    except (OSError, ValueError):
        cached = {}
    index = {'target': target, 'tools': {}}
    for name in TOOLS:
        info = MANIFEST['tools'][name]
        asset = info['files'][TARGETS[target]]
        version = asset.get('version', info['version'])
        suffix = '.exe' if 'windows' in target else ''
        rules = asset.get('bundle')
        folder = bool(rules and rules.get('keep_folder'))
        out = resources / name if folder else binaries / f'{name}-{target}{suffix}'
        old = cached.get('tools', {}).get(name, {}) if cached.get('target') == target else {}
        if old.get('version') == version and old.get('archive_sha256') == asset['sha256'] and old.get('files') and all(
                (out / rel if folder else out).is_file() and sha256(out / rel if folder else out) == digest
                for rel, digest in old['files'].items()):
            index['tools'][name] = old
            continue
        with tempfile.TemporaryDirectory(prefix='nezzel-tools-') as tmp:
            work = Path(tmp)
            archive = work / 'asset'
            checked_download(asset, archive)
            source = unpack(archive, asset, work, name)
            if source.is_dir():
                if out.exists():
                    shutil.rmtree(out)
                shutil.copytree(source, out)
                files = {p.relative_to(out).as_posix(): sha256(p) for p in sorted(out.rglob('*')) if p.is_file()}
                entry = rules.get('rename_entry_to', rules['entry'])
            else:
                temporary = out.with_name(out.name + '.tmp')
                shutil.copyfile(source, temporary)
                temporary.chmod(0o755)
                temporary.replace(out)
                files = {name + suffix: sha256(out)}
                entry = name + suffix
            index['tools'][name] = dict(version=version, archive_sha256=asset['sha256'], entry=entry, files=files)
            print(f'{name}: verified {version} for {target}')
    # Cache metadata is NOT shipped. Installer signatures seal the unpacked payload.
    temporary = index_path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(index, sort_keys=True) + '\n')
    temporary.replace(index_path)
    if 'linux' in target:
        stage_linux(target)
    return index


# Linux packages put externalBin in /usr/bin, over the distro's ffmpeg/aria2. tauri.linux.conf.json
# ships these as resources instead (<resources>/tools/<name>); stage them under arch-neutral names.
LINUX_SIDECARS = ('aria2c', 'gallery-dl', 'ffmpeg', 'ffprobe', 'deno')


def stage_linux(target):
    binaries = ROOT / 'src-tauri/binaries'
    staged = binaries / 'linux-tools'
    if staged.exists():
        shutil.rmtree(staged)
    staged.mkdir()
    for name in LINUX_SIDECARS:
        source = binaries / f'{name}-{target}'
        if source.is_file():
            shutil.copyfile(source, staged / name)
            (staged / name).chmod(0o755)


def sign_onedir(payload, identity):
    magic = {bytes.fromhex(value) for value in ('feedface', 'cefaedfe', 'feedfacf', 'cffaedfe', 'cafebabe', 'bebafeca', 'cafebabf', 'bfbafeca')}
    files = []
    for path in payload.rglob('*'):
        if path.is_file() and not path.is_symlink():
            with path.open('rb') as source:
                if source.read(4) in magic:
                    files.append(path)
    frameworks = [path for path in payload.rglob('*.framework') if path.is_dir()]
    entitlements = ROOT / 'scripts/tools-entitlements.plist'
    for path in sorted(files, key=lambda path: len(path.parts), reverse=True) + sorted(frameworks, key=lambda path: len(path.parts), reverse=True):
        subprocess.run(['codesign', '--force', '--sign', identity, '--options', 'runtime',
                        '--entitlements', str(entitlements), str(path)], check=True)
        subprocess.run(['codesign', '--verify', '--strict', str(path)], check=True)


# Tauri signs externalBin itself, but resource executables need explicit nested signing.
def sign_macos():
    if sys.platform != 'darwin':
        return
    resources = ROOT / 'src-tauri/binaries/tools'
    payload = resources / 'yt-dlp'
    if not payload.is_dir():
        raise ValueError('fetch installer tools before bundling')
    # Unset (local build without a certificate) means ad-hoc signing, like CI's unsigned beta.
    identity = os.environ.get('APPLE_SIGNING_IDENTITY') or '-'
    if identity == '-':
        # Upstream's yt-dlp onedir is already signed and runs as shipped; its flattened
        # Python.framework can't be re-signed as a bundle. Re-signing (hardened runtime)
        # only matters for notarized Developer ID builds.
        print('yt-dlp: ad-hoc build, keeping upstream signatures')
        return
    sign_onedir(payload, identity)
    # gallery-dl's onedir (fetch-gallery-dl.py): the app checks its entry against
    # gallery-dl.sha256, so record the hash of the signed bytes.
    gallery = resources / 'gallery-dl'
    if (gallery / 'gallery-dl').is_file():
        sign_onedir(gallery, identity)
        (ROOT / 'src-tauri/binaries/gallery-dl.sha256').write_text(sha256(gallery / 'gallery-dl') + '\n')
    # Preserve the verified local cache after codesign rewrites Mach-O bytes.
    index_path = resources / 'index.json'
    index = json.loads(index_path.read_text())
    index['tools']['yt-dlp']['files'] = {p.relative_to(payload).as_posix(): sha256(p) for p in sorted(payload.rglob('*')) if p.is_file()}
    temporary = index_path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(index, sort_keys=True) + '\n')
    temporary.replace(index_path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', default=os.environ.get('TAURI_ENV_TARGET_TRIPLE') or host_target(), choices=TARGETS)
    parser.add_argument('--sign-macos', action='store_true', help='sign nested yt-dlp code after certificate import, before bundling')
    args = parser.parse_args()
    if args.sign_macos:
        sign_macos()
    else:
        install(args.target)


if __name__ == '__main__':
    main()
