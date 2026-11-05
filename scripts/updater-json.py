#!/usr/bin/env python3
"""Write the release's one latest.json after every build leg has uploaded (publish.yml).

Build legs don't upload latest.json themselves: tauri-action reads, merges and re-uploads
it per leg, so concurrent legs can drop each other's entries. This job lists the release
assets once, pairs every updater bundle with its .sig, and fails unless every platform we
ship is present. Same shape as tauri-action's file: `{os}-{arch}` (primary: macOS .app,
Windows NSIS, Linux AppImage) plus `{os}-{arch}-{installer}`, with API asset URLs (the
updater sends Accept: application/octet-stream).

  GITHUB_TOKEN=... python3 scripts/updater-json.py --repo owner/name --tag app-v1.2.3 --notes "..."
"""
import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request

# Every key the publish.yml matrix ships: per OS and arch, the primary entry plus one per installer
# (macOS .app; Windows NSIS; Linux AppImage on the 24.04 legs, deb and rpm on the 22.04 legs).
# A missing installer key means a failed leg: don't publish a latest.json without it.
SHIPPED_INSTALLERS = {'darwin': ('app',), 'windows': ('nsis',), 'linux': ('appimage', 'deb', 'rpm')}
REQUIRED = tuple(
    key
    for system, installers in SHIPPED_INSTALLERS.items()
    for arch in ('aarch64', 'x86_64')
    for key in (f'{system}-{arch}', *(f'{system}-{arch}-{i}' for i in installers))
)

ARCH = {'aarch64': 'aarch64', 'arm64': 'aarch64', 'x64': 'x86_64', 'amd64': 'x86_64', 'x86_64': 'x86_64'}

# (suffix of the updater bundle, os, installer key, primary for {os}-{arch}?)
KINDS = (
    ('.app.tar.gz', 'darwin', 'app', True),
    ('-setup.exe', 'windows', 'nsis', True),
    ('.msi', 'windows', 'wix', False),
    ('.AppImage', 'linux', 'appimage', True),
    ('.deb', 'linux', 'deb', False),
    ('.rpm', 'linux', 'rpm', False),
)


def classify(name):
    """Bundle file name -> (os, arch, installer, primary), or None when it's no updater bundle."""
    for suffix, system, installer, primary in KINDS:
        if name.endswith(suffix):
            stem = name[: -len(suffix)]
            # Nezzel_1.0_aarch64.app.tar.gz, Nezzel_1.0_x64-setup.exe, Nezzel-1.0-1.x86_64.rpm
            found = re.findall(r'(?:^|[_.-])(aarch64|arm64|x86_64|amd64|x64)$', stem)
            arch = ARCH.get(found[-1]) if found else None
            return (system, arch, installer, primary) if arch else None
    return None


def build_manifest(assets, signature_of, version, notes, pub_date):
    """assets: [(name, api_url)]; signature_of(name) -> .sig text. Pure: no network."""
    names = {name for name, _ in assets}
    platforms = {}
    for name, url in sorted(assets):
        kind = classify(name)
        if not kind or name + '.sig' not in names:
            continue
        system, arch, installer, primary = kind
        entry = {'signature': signature_of(name + '.sig').strip(), 'url': url}
        platforms[f'{system}-{arch}-{installer}'] = entry
        if primary:
            platforms[f'{system}-{arch}'] = entry
    # Windows without NSIS falls back to the MSI, as tauri-action does.
    for arch in ('x86_64', 'aarch64'):
        if f'windows-{arch}' not in platforms and f'windows-{arch}-wix' in platforms:
            platforms[f'windows-{arch}'] = platforms[f'windows-{arch}-wix']
    return {'version': version, 'notes': notes, 'pub_date': pub_date, 'platforms': platforms}


def missing(manifest):
    return [key for key in REQUIRED if key not in manifest['platforms']]


def gh(*args):
    return subprocess.check_output(['gh', 'api', *args], text=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--repo', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--notes', default='')
    args = parser.parse_args()
    version = args.tag.removeprefix('app-v')
    releases = json.loads(gh('--paginate', '--slurp', f'repos/{args.repo}/releases'))
    matches = [r for page in releases for r in page if r['tag_name'] == args.tag]
    if len(matches) != 1:
        sys.exit(f'FAIL expected one release tagged {args.tag}, found {len(matches)}')
    release = matches[0]
    raw = json.loads(gh('--paginate', '--slurp', f"repos/{args.repo}/releases/{release['id']}/assets?per_page=100"))
    assets = [a for page in raw for a in page]
    by_name = {a['name']: a for a in assets}

    def signature_of(name):
        request = urllib.request.Request(by_name[name]['url'], headers={
            'Accept': 'application/octet-stream',
            'Authorization': f"Bearer {os.environ['GITHUB_TOKEN']}",
        })
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.read().decode()

    pub_date = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='milliseconds').replace('+00:00', 'Z')
    manifest = build_manifest([(a['name'], a['url']) for a in assets], signature_of, version, args.notes, pub_date)
    for key in sorted(manifest['platforms']):
        print(f"{key}: {manifest['platforms'][key]['url'].rsplit('/', 1)[-1]}")
    gaps = missing(manifest)
    if gaps:
        sys.exit('FAIL latest.json is missing ' + ', '.join(gaps))
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, 'latest.json')
        with open(path, 'w') as out:
            json.dump(manifest, out, indent=2)
        subprocess.run(['gh', 'release', 'upload', args.tag, path, '--clobber', '--repo', args.repo], check=True)
    print('PASS latest.json lists every shipped platform')


if __name__ == '__main__':
    main()
