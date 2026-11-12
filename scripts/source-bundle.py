#!/usr/bin/env python3
"""Bundle pinned third-party sources; never discover newer tool releases.

Usage: sh scripts/source-bundle.sh <version> <outdir> [--dry-run]
       sh scripts/source-bundle.sh --checksums-only <downloaded-release-assets>
"""
import argparse
import gzip
import hashlib
import http.client
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
FFMPEG_SIGNING_FINGERPRINT = 'FCF986EA15E6E293A5644F10B4322F04D67658D8'
RIEDL_REPO = 'https://git.martin-riedl.de/ffmpeg/build-script'
# Verified against both dated macOS build directories and the dated Windows/Linux recipe.
# Riedl fetched x264 master; this was its tip at both build timestamps (see archive README).
X264_REVISION = '0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee'
X264_SOURCE = dict(component='x264', version=X264_REVISION,
                   url=f'https://code.videolan.org/videolan/x264/-/archive/{X264_REVISION}/x264-{X264_REVISION}.tar.gz',
                   path=f'sources/x264-{X264_REVISION}.tar.gz',
                   sha256='d0967a1348c85dfde363bb52610403be898171493100561efa0dd05d5fd1ae50')
RIEDL_BUILDS = {'1789931006_9.0.2', '1789931890_9.0.2'}
RIEDL_RECIPE = 'f63b8aab8f5ce1a067da86ba69e34a36a7e217e5'
RIEDL_RECIPE_SHA256 = '025ae571e47d09bc7fe74fceae7f7b0b3b73bf304849133e24d5c4581e592473'
FFMPEG_BUILDS_TAG = 'autobuild-2026-11-01-19-06'
LIBRARY_CHECKSUMS = {
    ('libklvanc', '1.6.0'): '5076ca48455a4ef4ead2cd880ba189b21937a9ad8fd458adfc133d7bb1c948c3',
}
# URL conventions only; all versions come from the pinned provider's versions.txt.
# Git-generated archives have no separate published upstream checksum.
LIBRARY_URLS = {
    'aom': 'https://aomedia.googlesource.com/aom/+archive/v{version}.tar.gz',
    'libass': 'https://github.com/libass/libass/archive/refs/tags/{version}.tar.gz',
    'libbluray': 'https://code.videolan.org/videolan/libbluray/-/archive/{version}/libbluray-{version}.tar.gz',
    'dav1d': 'https://code.videolan.org/videolan/dav1d/-/archive/{version}/dav1d-{version}.tar.gz',
    'fontconfig': 'https://gitlab.freedesktop.org/fontconfig/fontconfig/-/archive/{version}/fontconfig-{version}.tar.gz',
    'freetype': 'https://gitlab.freedesktop.org/freetype/freetype/-/archive/VER-{dashed}/freetype-VER-{dashed}.tar.gz',
    'fribidi': 'https://github.com/fribidi/fribidi/archive/refs/tags/v{version}.tar.gz',
    'harfbuzz': 'https://github.com/harfbuzz/harfbuzz/archive/refs/tags/{version}.tar.gz',
    'libklvanc': 'https://github.com/stoth68000/libklvanc/archive/refs/tags/vid.obe.{version}.tar.gz',
    'lame': 'https://downloads.sourceforge.net/project/lame/lame/{version}/lame-{version}.tar.gz',
    'openh264': 'https://github.com/cisco/openh264/archive/refs/tags/v{version}.tar.gz',
    'openJPEG': 'https://github.com/uclouvain/openjpeg/archive/refs/tags/v{version}.tar.gz',
    'openssl': 'https://github.com/openssl/openssl/archive/refs/tags/openssl-{version}.tar.gz',
    'opus': 'https://github.com/xiph/opus/archive/refs/tags/v{version}.tar.gz',
    'rav1e': 'https://github.com/xiph/rav1e/archive/refs/tags/v{version}.tar.gz',
    'sdl': 'https://github.com/libsdl-org/SDL/archive/refs/tags/release-{version}.tar.gz',
    'snappy': 'https://github.com/google/snappy/archive/refs/tags/{version}.tar.gz',
    'srt': 'https://github.com/Haivision/srt/archive/refs/tags/v{version}.tar.gz',
    'svt-av1': 'https://gitlab.com/AOMediaCodec/SVT-AV1/-/archive/v{version}/SVT-AV1-v{version}.tar.gz',
    'libtheora': 'https://gitlab.xiph.org/xiph/theora/-/archive/v{version}/theora-v{version}.tar.gz',
    'libvmaf': 'https://github.com/Netflix/vmaf/archive/refs/tags/v{version}.tar.gz',
    'libvorbis': 'https://github.com/xiph/vorbis/archive/refs/tags/v{version}.tar.gz',
    'vpx': 'https://github.com/webmproject/libvpx/archive/refs/tags/v{version}.tar.gz',
    'vvenc': 'https://github.com/fraunhoferhhi/vvenc/archive/refs/tags/v{version}.tar.gz',
    'libwebp': 'https://github.com/webmproject/libwebp/archive/refs/tags/v{version}.tar.gz',
    'x265': 'https://bitbucket.org/multicoreware/x265_git/get/{version}.tar.gz',
    'libxml2': 'https://gitlab.gnome.org/GNOME/libxml2/-/archive/v{version}/libxml2-v{version}.tar.gz',
    'zimg': 'https://github.com/sekrit-twc/zimg/archive/refs/tags/release-{version}.tar.gz',
    'zlib': 'https://github.com/madler/zlib/archive/refs/tags/v{version}.tar.gz',
    'zvbi': 'https://downloads.sourceforge.net/project/zapping/zvbi/{version}/zvbi-{version}.tar.bz2',
}
LOCAL_FILES = {
    'src-tauri/src/binaries/manifest.json': 'manifests/tools.json',
    'scripts/gallery-dl-manifest.json': 'manifests/gallery-dl.json',
    'scripts/aria2-manifest.json': 'manifests/aria2.json',
    'scripts/fetch-tools.py': 'recipes/nezzel/fetch-tools.py',
    'scripts/fetch-gallery-dl.py': 'recipes/nezzel/fetch-gallery-dl.py',
    'scripts/gallery-dl-build-requirements.txt': 'recipes/nezzel/gallery-dl-build-requirements.txt',
    'scripts/fetch-aria2.py': 'recipes/nezzel/fetch-aria2.py',
    'scripts/ffmpeg-COPYING.GPLv3': 'licenses/ffmpeg-COPYING.GPLv3',
    'scripts/gallery-dl-LICENSE': 'licenses/gallery-dl-LICENSE',
    'scripts/gallery-dl-bundled-NOTICES.txt': 'licenses/gallery-dl-bundled-NOTICES.txt',
    'scripts/aria2-COPYING': 'licenses/aria2-COPYING',
    'licenses/tools-NOTICES.txt': 'licenses/tools-NOTICES.txt',
}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def library_sources(text, x264_revision=None):
    sources, missing = [], []
    for line in text.splitlines():
        match = re.fullmatch(r'([A-Za-z][A-Za-z0-9_-]*)\s+([0-9][A-Za-z0-9.+_-]*)\s*', line)
        if not match:
            continue
        name, version = match.groups()
        if (name, version, x264_revision) == ('x264', '0.165.x', X264_REVISION):
            sources.append(dict(X264_SOURCE))
        elif not re.fullmatch(r'[0-9]+(?:\.[0-9]+)+(?:[-_][0-9]+)?', version):
            missing.append(f'{name} {version}: no exact source revision')
        elif name not in LIBRARY_URLS:
            missing.append(f'{name} {version}: no source URL mapping')
        else:
            url = LIBRARY_URLS[name].format(version=version, dashed=version.replace('.', '-'))
            extension = '.tar.bz2' if url.endswith('.tar.bz2') else '.tar.gz'
            source = dict(component=name, version=version, url=url,
                          path=f'sources/riedl-libraries/{name}-{version}{extension}')
            if (name, version) in LIBRARY_CHECKSUMS:
                source['sha256'] = LIBRARY_CHECKSUMS[name, version]
            sources.append(source)
    return sources, missing


def plan(version, root=ROOT):
    if not re.fullmatch(r'[0-9]+(?:\.[0-9]+)+(?:[-+][A-Za-z0-9.-]+)?', version):
        raise ValueError('Invalid version (omit the v prefix)')
    tools = json.loads((root / 'src-tauri/src/binaries/manifest.json').read_text())['tools']
    downloads, binaries = {}, []

    def add(component, version, url, path, **verification):
        row = dict(component=component, version=version, url=url, path=path, **verification)
        if path in downloads and downloads[path] != row:
            raise ValueError('Conflicting source path: ' + path)
        downloads[path] = row

    for name in ('aria2', 'gallery-dl'):
        manifest = json.loads((root / f'scripts/{name}-manifest.json').read_text())
        source = manifest['source']
        if not re.fullmatch('[0-9a-f]{64}', source['sha256']):
            raise ValueError('Missing pinned source checksum: ' + name)
        add(name, manifest['version'], source['url'],
            'sources/' + unquote(urlsplit(source['url']).path.rsplit('/', 1)[1]), sha256=source['sha256'])
        binaries.append(dict(component=name, version=manifest['version'], license=manifest['license'],
                             provenance=manifest))

    for platform, asset in tools['ffmpeg']['files'].items():
        probe = tools['ffprobe']['files'][platform]
        version_ff = asset.get('version', tools['ffmpeg']['version'])
        if probe.get('version', tools['ffprobe']['version']) != version_ff:
            raise ValueError('ffmpeg/ffprobe source versions differ: ' + platform)
        binaries.append(dict(component='ffmpeg/ffprobe', platform=platform, version=version_ff,
                             license='GPL-3.0-or-later', ffmpeg=asset, ffprobe=probe))
        url = asset['url']
        if url.startswith('https://ffmpeg.martin-riedl.de/download/macos/'):
            if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', version_ff):
                raise ValueError('Riedl asset must pin an FFmpeg release')
            source = f'https://ffmpeg.org/releases/ffmpeg-{version_ff}.tar.xz'
            path = f'sources/ffmpeg-{version_ff}.tar.xz'
            add('ffmpeg', version_ff, source, path, signature=path + '.asc')
            add('ffmpeg-signature', version_ff, source + '.asc', path + '.asc')
            add('ffmpeg-key', FFMPEG_SIGNING_FINGERPRINT, 'https://ffmpeg.org/ffmpeg-devel.asc', 'verification/ffmpeg-devel.asc')
            base = url.rsplit('/', 1)[0]
            if probe['url'].rsplit('/', 1)[0] != base:
                raise ValueError('ffmpeg/ffprobe Riedl build directories differ: ' + platform)
            timestamp = re.fullmatch(r'([0-9]+)_' + re.escape(version_ff), base.rsplit('/', 1)[1])
            if not timestamp:
                raise ValueError('Riedl build directory must pin its timestamp/version')
            if base.rsplit('/', 1)[1] not in RIEDL_BUILDS:
                raise ValueError('Unverified Riedl source pins: ' + base)
            add('riedl-recipe', RIEDL_RECIPE, f'{RIEDL_REPO}/archive/{RIEDL_RECIPE}.tar.gz',
                f'recipes/riedl/build-script-{RIEDL_RECIPE}.tar.gz', sha256=RIEDL_RECIPE_SHA256)
            # The immutable binary build directory supplies its configure flags/library versions.
            add('riedl-build-info', version_ff, base + '/versions.txt', f'recipes/riedl/{platform}-versions.txt',
                x264_revision=X264_REVISION)
            add(**X264_SOURCE)
        else:
            match = re.fullmatch(r'https://github.com/yt-dlp/FFmpeg-Builds/releases/download/(autobuild-[0-9-]+)/ffmpeg-.*-gpl\.(?:zip|tar\.xz)', url)
            revision = re.fullmatch(r'N-[0-9]+-g([0-9a-f]{10,40})', version_ff)
            if not match or not revision:
                raise ValueError('Unrecognized pinned FFmpeg provider/version: ' + url)
            if probe['url'] != url:
                raise ValueError('ffprobe must come from the same provider archive: ' + platform)
            tag, commit = match[1], revision[1]
            if tag != FFMPEG_BUILDS_TAG:
                raise ValueError('Unverified FFmpeg-Builds x264 pin: ' + tag)
            add(**X264_SOURCE)
            add('ffmpeg', version_ff, f'https://github.com/FFmpeg/FFmpeg/archive/{commit}.tar.gz',
                f'sources/ffmpeg-{commit}.tar.gz')
            add('ffmpeg-builds-recipe', tag,
                f'https://github.com/yt-dlp/FFmpeg-Builds/archive/refs/tags/{tag}.tar.gz',
                f'recipes/FFmpeg-Builds-{tag}.tar.gz')
    rows = sorted(downloads.values(), key=lambda row: row['path'])
    return dict(archive=f'Nezzel-{version}-SOURCES.tar.gz', tag='app-v' + version,
                prefix=f'Nezzel-{version}-SOURCES', downloads=rows, binaries=binaries,
                derived_downloads={
                    'x264_revision': X264_REVISION,
                    'riedl_libraries': LIBRARY_URLS,
                    'versions_from': 'recipes/riedl/*-versions.txt (fetched only in live mode)',
                    'layout': ['sources/riedl-libraries/{library}-{version}.tar.{gz,bz2}'],
                },
                layout=sorted(['README.md', 'SHA256SUMS', 'provenance.json', *LOCAL_FILES.values(),
                               *(row['path'] for row in rows)]))


def download(row, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(row['url'], headers={'User-Agent': 'Nezzel-source-bundle/1.0'})
    # Some hosts (ffmpeg.org) reset connections now and then: retry with backoff, name the URL on failure.
    for attempt in range(1, 6):
        try:
            with urllib.request.urlopen(request, timeout=120) as response, destination.open('wb') as out:
                shutil.copyfileobj(response, out)
            break
        except urllib.error.HTTPError as error:
            if error.code < 500 and error.code != 429:
                raise ValueError(f'HTTP {error.code}: {row["url"]}') from error
            failure = error
        except (OSError, http.client.HTTPException) as error:
            failure = error
        if attempt == 5:
            raise ValueError(f'Download failed after {attempt} tries: {row["url"]} ({failure})') from failure
        print(f'retry {attempt} for {row["url"]}: {failure}', file=sys.stderr)
        time.sleep(2 ** attempt)
    if row.get('sha256') and digest(destination) != row['sha256']:
        destination.unlink()
        raise ValueError('Checksum mismatch: ' + row['url'])
    if row['path'].endswith(('.tar.gz', '.tar.xz', '.tar.bz2')):
        try:
            with tarfile.open(destination) as archive:
                if not any(member.isfile() for member in archive):
                    raise ValueError('Source archive contains no files: ' + row['url'])
        except (tarfile.TarError, ValueError) as error:
            destination.unlink()
            raise ValueError('Invalid source archive: ' + row['url']) from error


def verify_signatures(bundle, stage, work):
    signed = [row for row in bundle['downloads'] if row.get('signature')]
    if not signed:
        return
    home = work / 'gnupg'
    home.mkdir(mode=0o700)
    command = ['gpg', '--batch', '--homedir', str(home)]
    subprocess.run([*command, '--import', str(stage / 'verification/ffmpeg-devel.asc')],
                   check=True, capture_output=True)
    for row in signed:
        result = subprocess.run([*command, '--status-fd', '1', '--verify',
                                 str(stage / row['signature']), str(stage / row['path'])],
                                check=True, text=True, capture_output=True)
        if not any(line.startswith('[GNUPG:] VALIDSIG ') and FFMPEG_SIGNING_FINGERPRINT in line.split()
                   for line in result.stdout.splitlines()):
            raise ValueError('FFmpeg signature is not from the expected release key')


def provider_sources(bundle, stage):
    """Resolve versioned linked-library sources from the pinned provider metadata."""
    additions, missing = {}, []
    for row in list(bundle['downloads']):
        if row['component'] == 'riedl-build-info':
            sources, unresolved = library_sources((stage / row['path']).read_text(), row.get('x264_revision'))
            if not sources:
                raise ValueError('No linked library versions found in ' + row['url'])
            missing.extend(f"{row['path']}: {item}" for item in unresolved)
            for source in sources:
                if not any(existing['path'] == source['path'] for existing in bundle['downloads']):
                    additions[source['path']] = source
    for row in sorted(additions.values(), key=lambda item: item['path']):
        destination = stage / row['path']
        try:
            download(row, destination)
        except (OSError, ValueError) as error:
            if row.get('sha256'):
                raise
            destination.unlink(missing_ok=True)
            missing.append(f"{row['component']} {row['version']}: {row['url']} ({error})")
            continue
        row['downloaded_sha256'] = digest(destination)
        bundle['downloads'].append(row)
        bundle['layout'].append(row['path'])
    bundle['unresolved_sources'] = sorted(set(missing))
    for item in bundle['unresolved_sources']:
        print('WARNING unresolved source: ' + item, file=sys.stderr)


def checksums(directory, recursive=False):
    paths = sorted(directory.rglob('*') if recursive else directory.iterdir())
    lines = []
    for path in paths:
        if path.name in ('SHA256SUMS', '.SHA256SUMS.tmp') or path.is_dir():
            continue
        name = path.relative_to(directory).as_posix()
        if path.is_symlink() or any(c in name for c in '\n\r\\'):
            raise ValueError('Unsafe checksum entry: ' + name)
        lines.append(digest(path) + '  ' + name + '\n')
    if not lines:
        raise ValueError('No assets to hash')
    temporary = directory / '.SHA256SUMS.tmp'
    temporary.write_text(''.join(lines), encoding='utf-8')
    temporary.replace(directory / 'SHA256SUMS')


def readme(bundle, repo):
    lines = [f"# Nezzel {bundle['tag']} — third-party source", '',
             'This archive accompanies the desktop installers. Sources are unmodified upstream',
             'archives; Nezzel build scripts (including the aria2 patch), manifests and licence',
             'texts are included alongside them. ffmpeg and ffprobe share the same FFmpeg source.', '',
             f"App source (AGPL-3.0-or-later): https://github.com/{repo}/tree/{bundle['tag']}",
             'The app source is published in that tagged repository, not duplicated here.', '',
             '## Shipped tools', '']
    for row in bundle['binaries']:
        lines.append(f"- {row['component']} {row['version']} ({row['license']})" +
                     (f" — {row['platform']}; binary: {row['ffmpeg']['url']}; ffprobe: {row['ffprobe']['url']}"
                      if 'platform' in row else '; binary origins/targets: see manifests and provenance.json'))
    lines += ['', '## Build information and verification', '',
              'macOS: Martin Riedl builds. The exact FFmpeg release source and per-build',
              'versions.txt (configure flags and dependency versions) are included.',
              'Linked-library archives are resolved from those per-build versions, without using latest.',
              'Riedl has no recipe tags or binary-to-recipe commit mapping. The recipe is the last',
              f'main-branch commit at/before both binary build timestamps: {RIEDL_RECIPE}.',
              'Its archive SHA-256 is pinned locally; no mutable recipe/history lookup is used.',
              'This is a historical snapshot, not a provider-certified exact recipe commit.',
              'Windows/Linux: yt-dlp/FFmpeg-Builds GPL builds. FFmpeg source is taken at the',
              'commit embedded in the binary version; recipes/ contains the provider dated tag',
              '(scripts, patches and dependency recipes). No latest-release lookup is used.',
              f'x264 source: {X264_REVISION} (shared by these pinned providers).',
              'Windows/Linux evidence: scripts.d/50-x264.sh in the dated provider recipe pins that SHA.',
              'macOS evidence: script/build-x264.sh in the pinned Riedl recipe downloads master.',
              'The upstream x264 master history at both build timestamps ends at that SHA',
              '(2025-09-10); no later commits precede either 2026-09-20 macOS build.',
              'History: https://code.videolan.org/api/v4/projects/videolan%2Fx264/repository/commits'
              '?ref_name=master&until=2026-09-20T19:18:10Z&per_page=1',
              'This macOS revision is inferred from upstream history, not certified by Riedl;',
              'both binary x264 version strings omit a revision (core 0000).',
              'libklvanc 1.6.0 uses upstream tag vid.obe.1.6.0, as in the Riedl recipe.',
              'gallery-dl: official assets or native Python builds per its manifest; native build',
              'recipe and hash-locked Python requirements are in recipes/nezzel/.',
              'aria2: official Windows x64 binary or native AppleTLS/static OpenSSL builds;',
              'recipes/nezzel/fetch-aria2.py includes the Linux OpenSSL patch and configure flags.',
              'Windows ARM64 has no aria2 sidecar (disabled stub), per its manifest.', '',
              'aria2/gallery-dl source SHA-256 values are checked against the build manifests.',
              'x264, libklvanc 1.6.0 and the Riedl recipe are checked against local SHA-256 pins.',
              f'FFmpeg release signatures are verified with primary key {FFMPEG_SIGNING_FINGERPRINT}.',
              'Git provider snapshots/build metadata have no upstream checksum; their downloaded',
              'SHA-256 values are recorded in provenance.json and the inner SHA256SUMS.',
              'Retain licence and dependency notices contained within upstream source archives.',
              'The release-level SHA256SUMS covers all uploaded assets except itself.', '']
    lines += ['## Unresolved source inputs', '',
              *(bundle.get('unresolved_sources') or ['None reported by the source resolver.']), '',
              'Unresolved entries need provider clarification before claiming complete corresponding',
              'source coverage. A recipe alone does not supply missing linked-library source.', '']
    return '\n'.join(lines)


def build(bundle, output, repo, root=ROOT):
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.nezzel-sources-', dir=output) as temporary:
        work = Path(temporary)
        stage = work / bundle['prefix']
        stage.mkdir()
        for source, target in LOCAL_FILES.items():
            destination = stage / target
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / source, destination)
        for row in bundle['downloads']:
            destination = stage / row['path']
            download(row, destination)
            row['downloaded_sha256'] = digest(destination)
        verify_signatures(bundle, stage, work)
        provider_sources(bundle, stage)
        (stage / 'README.md').write_text(readme(bundle, repo), encoding='utf-8')
        (stage / 'provenance.json').write_text(json.dumps(bundle, indent=2) + '\n', encoding='utf-8')
        checksums(stage, recursive=True)
        # Stable metadata and ordering, like Android's source bundle. Never extract upstream tarballs.
        target = work / bundle['archive']
        with target.open('wb') as dest, gzip.GzipFile(filename='', mode='wb', fileobj=dest, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode='w', format=tarfile.PAX_FORMAT) as tar:
                for path in sorted(stage.rglob('*')):
                    if path.is_file():
                        member = tarfile.TarInfo(bundle['prefix'] + '/' + path.relative_to(stage).as_posix())
                        member.size, member.mode = path.stat().st_size, 0o644
                        with path.open('rb') as source:
                            tar.addfile(member, source)
        target.replace(output / bundle['archive'])
    print(output / bundle['archive'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('version', nargs='?')
    parser.add_argument('outdir', nargs='?', type=Path)
    parser.add_argument('--dry-run', action='store_true', help='print JSON URLs/layout, no network or writes')
    parser.add_argument('--checksums-only', type=Path, help='hash every downloaded release asset except SHA256SUMS')
    parser.add_argument('--repo', default='ahmed-abdat/nezzel-desktop')
    args = parser.parse_args()
    if args.checksums_only:
        if args.dry_run or args.version or args.outdir:
            parser.error('--checksums-only cannot be combined with bundle arguments')
        checksums(args.checksums_only)
        return
    if not args.version or not args.outdir:
        parser.error('version and outdir are required')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repo):
        parser.error('invalid GitHub repository')
    bundle = plan(args.version)
    if args.dry_run:
        print(json.dumps(bundle, indent=2))
    else:
        build(bundle, args.outdir, args.repo)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit('error: ' + str(error))
