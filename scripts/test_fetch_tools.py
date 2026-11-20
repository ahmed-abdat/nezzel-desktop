"""Offline bundle preparation contracts; run by the Rust CI jobs."""
import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path
import tempfile
import tarfile
import io
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('fetch_tools', Path(__file__).with_name('fetch-tools.py'))
bootstrap = importlib.util.module_from_spec(spec)


class FetchToolsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec.loader.exec_module(bootstrap)

    def test_manifest_keygen_keeps_disposable_test_key_private_and_refuses_overwrite(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory(prefix='nezzel-test-key-') as tmp:
            path = Path(tmp) / 'disposable.key'
            generated = subprocess.run(['node', 'scripts/manifest-keygen.mjs', str(path)],
                                       cwd=root, capture_output=True, text=True)
            self.assertEqual(generated.returncode, 0, generated.stderr)
            public = generated.stdout.strip()
            self.assertEqual(len(bytes.fromhex(public)), 32)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            private = path.read_bytes()
            self.assertNotIn(private.decode(), generated.stdout + generated.stderr)
            check = subprocess.run(['node', '--input-type=module', '-e', r"""
import assert from 'node:assert/strict';
import {createPrivateKey, createPublicKey} from 'node:crypto';
import {readFileSync} from 'node:fs';
const key = createPrivateKey({key: Buffer.from(readFileSync(process.argv[1], 'utf8'), 'base64'), format: 'der', type: 'pkcs8'});
assert.equal(Buffer.from(createPublicKey(key).export({format: 'jwk'}).x, 'base64url').toString('hex'), process.argv[2]);
""", str(path), public], cwd=root, capture_output=True, text=True)
            self.assertEqual(check.returncode, 0, check.stderr)
            again = subprocess.run(['node', 'scripts/manifest-keygen.mjs', str(path)],
                                   cwd=root, capture_output=True, text=True)
            self.assertNotEqual(again.returncode, 0)
            self.assertEqual(again.stdout, '')
            self.assertEqual(path.read_bytes(), private)

    def test_manifest_bump_rejects_unhashable_assets_and_signs_exact_bytes(self):
        result = subprocess.run(['node', '--input-type=module', '-e', r"""
import assert from 'node:assert/strict';
import { createPublicKey, verify } from 'node:crypto';
import { hashAsset, signManifest, updateGithubTool } from './scripts/manifest-bump.mjs';
const bytes = Buffer.from('abc');
assert.equal(await hashAsset('https://github.com/test/asset', async () => new Response(bytes)),
  'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
for (const response of [new Response(null), new Response('', {status: 404}),
  new Response(new ReadableStream({start(c) { c.error(new Error('interrupted')); }}))]) {
  await assert.rejects(hashAsset('https://github.com/test/asset', async () => response));
}
const tool = {version: 'old', files: {test: {url: 'https://github.com/test/tool/releases/download/old/tool.zip',
  sha256: 'old', bundle: {entry: 'tool', keep_folder: true}}}};
await updateGithubTool(tool, {tag_name: 'new', assets: [
  {name: 'tool.zip', browser_download_url: 'https://github.com/test/tool/releases/download/new/tool.zip'}]},
  new Map(), async () => new Response(bytes));
assert.equal(tool.version, 'new');
assert.equal(tool.files.test.sha256, 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
assert.deepEqual(tool.files.test.bundle, {entry: 'tool', keep_folder: true});
await assert.rejects(updateGithubTool(structuredClone(tool), {tag_name: 'bad', assets: []}, new Map()));
// Deterministic test seed only; never a production key.
const key = Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.alloc(32, 1)]).toString('base64');
const signature = signManifest(bytes, key);
const {createPrivateKey} = await import('node:crypto');
const privateKey = createPrivateKey({key: Buffer.from(key, 'base64'), format: 'der', type: 'pkcs8'});
assert(verify(null, bytes, createPublicKey(privateKey), Buffer.from(signature, 'base64')));
assert(!verify(null, Buffer.from('changed'), createPublicKey(privateKey), Buffer.from(signature, 'base64')));
assert.throws(() => signManifest(bytes, 'bad'));
const publicHex = Buffer.from(createPublicKey(privateKey).export({format: 'jwk'}).x, 'base64url').toString('hex');
assert.equal(signManifest(bytes, key, publicHex), signature);
assert.throws(() => signManifest(bytes, key, '00'.repeat(32)));
assert.throws(() => signManifest(bytes, key, 'bad-public-key'));
process.env.GITHUB_TOKEN = 'disposable-fixture-token';
await hashAsset('https://github.com/test/asset', async (url, {headers}) => {
  assert.equal(headers.Authorization, undefined);
  return new Response(bytes);
});
"""], cwd=Path(__file__).resolve().parents[1], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_manifest_bump_preserves_ffmpeg_pins_without_requests(self):
        result = subprocess.run(['node', '--input-type=module', '-e', r"""
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {bump} from './scripts/manifest-bump.mjs';
const load = async path => JSON.parse(await readFile(path));
const input = await Promise.all(['src-tauri/src/binaries/manifest.json',
  'scripts/gallery-dl-manifest.json', 'scripts/aria2-manifest.json'].map(load));
input[0].tools['yt-dlp'].rollback = true;
for (const name of ['ffmpeg', 'ffprobe']) {
  for (const [platform, file] of Object.entries(input[0].tools[name].files)) {
    if (!platform.startsWith('darwin-')) {
      file.source = file.source || file.url;
      file.url = `https://github.com/ahmed-abdat/nezzel-desktop/releases/download/tools-ffmpeg-pin/${platform}.zip`;
    }
  }
}
const original = structuredClone(input);
const asset = (repo, tag, name) => ({name, browser_download_url: `https://github.com/${repo}/releases/download/${tag}/${name}`});
const release = (repo, tag, names) => ({tag_name: tag, assets: names.map(name => asset(repo, tag, name))});
const releases = {
  'yt-dlp/yt-dlp': release('yt-dlp/yt-dlp', '2026.11.11', Object.values(input[0].tools['yt-dlp'].files).map(f => f.url.split('/').pop())),
  'denoland/deno': release('denoland/deno', 'v2.9.9', Object.values(input[0].tools.deno.files).map(f => f.url.split('/').pop())),
  'mikf/gallery-dl': release('mikf/gallery-dl', 'v1.33.0', ['gallery_dl-1.33.0.tar.gz', 'gallery-dl.bin', 'gallery-dl.exe']),
  'aria2/aria2': release('aria2/aria2', 'release-1.38.0', ['aria2-1.38.0.tar.xz', 'aria2-1.38.0-win-64bit-build1.zip']),
};
const ffRequests = [];
const fetcher = async url => {
  if (/FFmpeg-Builds|martin-riedl|tools-ffmpeg/.test(url)) {
    ffRequests.push(url);
    throw new Error('FFmpeg must stay pinned');
  }
  const match = url.match(/repos\/(.+)\/releases\/latest$/);
  if (match) { assert(releases[match[1]], url); return Response.json(releases[match[1]]); }
  if (url.includes('/redirect/latest/')) {
    const final = url.replace('/redirect/latest/', '/download/').replace('/release/', '/123_9.1.0/');
    return Object.defineProperty(new Response(null), 'url', {value: final});
  }
  return new Response('abc');
};
const out = await bump(...input, fetcher);
assert.deepEqual(input, original);
const manifest = out['manifest.json'];
assert.equal(manifest.tools['yt-dlp'].version, '2026.11.11');
assert.equal(manifest.tools['yt-dlp'].rollback, undefined);
assert.deepEqual(ffRequests, []);
for (const name of ['ffmpeg', 'ffprobe']) assert.deepEqual(manifest.tools[name], original[0].tools[name]);
assert.deepEqual(manifest.tools['yt-dlp'].files['darwin-aarch64'].bundle, original[0].tools['yt-dlp'].files['darwin-aarch64'].bundle);
assert.equal(out['gallery-dl-manifest.json'].version, '1.33.0');
assert.equal(out['aria2-manifest.json'].version, '1.38.0');
assert.deepEqual(manifest.tools.AtomicParsley, original[0].tools.AtomicParsley);
for (const tool of ['yt-dlp', 'deno']) {
  for (const file of Object.values(manifest.tools[tool].files))
    assert.equal(file.sha256, 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
}
const partial = await bump(...input, async url => url.includes('denoland/deno')
  ? new Response('', {status: 502}) : fetcher(url));
assert.deepEqual(partial['manifest.json'].tools.deno, original[0].tools.deno);
assert.equal(partial['manifest.json'].tools['yt-dlp'].version, '2026.11.11');
const downloadFailure = await bump(...input, async url => url.endsWith('yt-dlp_win_arm64.zip')
  ? new Response('', {status: 502}) : fetcher(url));
assert.deepEqual(downloadFailure['manifest.json'].tools['yt-dlp'], original[0].tools['yt-dlp']);
assert.equal(downloadFailure['manifest.json'].tools.deno.version, 'v2.9.9');
await assert.rejects(bump(...input, async () => new Response('', {status: 502})), /Every tool source failed/);
assert.deepEqual(input, original);
"""], cwd=Path(__file__).resolve().parents[1], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_ffmpeg_mirror_rewrites_upstream_urls_and_keeps_hash_entry_and_source(self):
        result = subprocess.run(['node', '--input-type=module', '-e', r"""
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {mirrorUrl, planMirror} from './scripts/manifest-bump.mjs';
const up = 'https://github.com/yt-dlp/FFmpeg-Builds/releases/download/autobuild-2026-11-16-23-35/ffmpeg-N-1-gabc-win64-gpl.zip';
assert.deepEqual(mirrorUrl(up, 'o/r'), {upstreamTag: 'autobuild-2026-11-16-23-35', name: 'ffmpeg-N-1-gabc-win64-gpl.zip',
  tag: 'tools-ffmpeg-autobuild-2026-11-16-23-35',
  url: 'https://github.com/o/r/releases/download/tools-ffmpeg-autobuild-2026-11-16-23-35/ffmpeg-N-1-gabc-win64-gpl.zip'});
assert.throws(() => mirrorUrl('https://ffmpeg.martin-riedl.de/download/macos/arm64/1_9/ffmpeg.zip'), /Not a/);
const manifest = JSON.parse(await readFile('src-tauri/src/binaries/manifest.json'));
for (const tool of ['ffmpeg', 'ffprobe']) {
  for (const file of Object.values(manifest.tools[tool].files)) {
    if (file.source) file.url = file.source;
  }
}
const before = structuredClone(manifest);
const {manifest: next, archives} = planMirror(manifest);
assert.deepEqual(manifest, before);
assert.equal(archives.length, 4);
for (const name of ['ffmpeg', 'ffprobe']) {
  for (const [platform, file] of Object.entries(next.tools[name].files)) {
    const old = before.tools[name].files[platform];
    assert.equal(file.sha256, old.sha256);
    assert.equal(file.entry, old.entry);
    assert.deepEqual(file.bundle, old.bundle);
    if (platform.startsWith('darwin-')) assert.equal(file.url, old.url);
    else {
      assert(file.url.startsWith('https://github.com/ahmed-abdat/nezzel-desktop/releases/download/tools-ffmpeg-'));
      assert.equal(file.source, old.url);
    }
  }
}
assert.deepEqual(planMirror(next).archives, []);
"""], cwd=Path(__file__).resolve().parents[1], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_archive_and_raw_assets_keep_manifest_layout_and_cache_is_verified(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'yt-dlp.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                bundle.writestr('distribution/entry', b'ytdlp')
                bundle.writestr('distribution/_internal/library', b'python')
            deno = root / 'deno.zip'
            with zipfile.ZipFile(deno, 'w') as bundle:
                bundle.writestr('deno', b'deno')
            raw = root / 'ffmpeg'
            raw.write_bytes(b'ffmpeg')
            def asset(path, **extra):
                return dict(url=path.as_uri(), sha256=hashlib.sha256(path.read_bytes()).hexdigest(), **extra)
            files = {
                'yt-dlp': asset(archive, bundle=dict(keep_folder=True, folder_name='distribution', entry='entry', rename_entry_to='yt-dlp')),
                'deno': asset(deno, entry='deno'),
                'ffmpeg': asset(raw),
                'ffprobe': asset(raw),
            }
            manifest = {'tools': {name: {'version': '1', 'files': {'linux-x86_64': value}} for name, value in files.items()}}
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest):
                bootstrap.install('x86_64-unknown-linux-gnu')
                binary = root / 'src-tauri/binaries/ffmpeg-x86_64-unknown-linux-gnu'
                self.assertEqual(binary.read_bytes(), b'ffmpeg')
                resources = root / 'src-tauri/binaries/tools'
                self.assertEqual((resources / 'yt-dlp/_internal/library').read_bytes(), b'python')
                index = json.loads((resources / 'index.json').read_text())
                self.assertEqual(index['tools']['deno']['archive_sha256'], files['deno']['sha256'])
                self.assertEqual(index['tools']['yt-dlp']['entry'], 'yt-dlp')
                binary.write_bytes(b'bad cache')
                bootstrap.install('x86_64-unknown-linux-gnu')
                self.assertEqual(binary.read_bytes(), b'ffmpeg')
                with patch.object(bootstrap.urllib.request, 'urlopen', side_effect=AssertionError('cache unexpectedly downloaded')):
                    bootstrap.install('x86_64-unknown-linux-gnu')
                # Linux packages ship tools as resources (never /usr/bin), arch-neutral and executable.
                staged = root / 'src-tauri/binaries/linux-tools'
                self.assertEqual(sorted(p.name for p in staged.iterdir()), ['deno', 'ffmpeg', 'ffprobe'])
                self.assertTrue(all(p.stat().st_mode & 0o111 for p in staged.iterdir()))
                (root / 'src-tauri/binaries/aria2c-x86_64-unknown-linux-gnu').write_bytes(b'aria2')
                (staged / 'leftover').write_bytes(b'old')
                with patch.object(bootstrap.urllib.request, 'urlopen', side_effect=AssertionError('cache unexpectedly downloaded')):
                    bootstrap.install('x86_64-unknown-linux-gnu')
                self.assertEqual(sorted(p.name for p in staged.iterdir()), ['aria2c', 'deno', 'ffmpeg', 'ffprobe'])

    def test_windows_arm64_archive_entries_use_exe_sidecars_and_preserve_onedir(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'windows.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                bundle.writestr('build/bin/ffmpeg.exe', b'ffmpeg')
                bundle.writestr('build/bin/ffprobe.exe', b'ffprobe')
                bundle.writestr('deno.exe', b'deno')
                bundle.writestr('yt-dlp_arm64.exe', b'ytdlp')
                bundle.writestr('_internal/python.dll', b'python')
            asset = dict(url=archive.as_uri(), sha256=hashlib.sha256(archive.read_bytes()).hexdigest())
            files = {
                'ffmpeg': dict(asset, entry='build/bin/ffmpeg.exe'),
                'ffprobe': dict(asset, entry='build/bin/ffprobe.exe'),
                'deno': dict(asset, entry='deno.exe'),
                'yt-dlp': dict(asset, bundle=dict(keep_folder=True, folder_name='yt-dlp', entry='yt-dlp_arm64.exe', rename_entry_to='yt-dlp.exe')),
            }
            manifest = {'tools': {name: {'version': '1', 'files': {'windows-aarch64': asset}} for name, asset in files.items()}}
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest):
                bootstrap.install('aarch64-pc-windows-msvc')
            binaries = root / 'src-tauri/binaries'
            self.assertEqual((binaries / 'ffprobe-aarch64-pc-windows-msvc.exe').read_bytes(), b'ffprobe')
            self.assertEqual((binaries / 'tools/yt-dlp/yt-dlp.exe').read_bytes(), b'ytdlp')
            self.assertEqual((binaries / 'tools/yt-dlp/_internal/python.dll').read_bytes(), b'python')

    def test_windows_shared_pair_stages_once_and_verifies_every_file(self):
        for arch in ('x86_64', 'aarch64'):
            with self.subTest(arch=arch), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                archive = root / 'shared.zip'
                with zipfile.ZipFile(archive, 'w') as bundle:
                    for name in ('ffmpeg.exe', 'ffprobe.exe', 'codec.dll', 'extra.DLL', 'ffplay.exe'):
                        bundle.writestr('build/bin/' + name, name.encode())
                    bundle.writestr('build/doc/manual', b'doc')
                asset = dict(url=archive.as_uri(), sha256=bootstrap.sha256(archive))
                files = {name: dict(asset, bundle=dict(keep_folder=True, folder_name='build/bin', entry=name + '.exe'))
                         for name in ('ffmpeg', 'ffprobe')}
                platform = 'windows-' + arch
                manifest = {'tools': {name: {'version': '1', 'files': {platform: file}} for name, file in files.items()}}
                with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest), \
                        patch.object(bootstrap, 'TOOLS', ('ffmpeg', 'ffprobe')), \
                        patch.object(bootstrap, 'checked_download', wraps=bootstrap.checked_download) as download:
                    target = arch + '-pc-windows-msvc'
                    index = bootstrap.install(target)['tools']
                    folder = root / 'src-tauri/binaries/tools/ffmpeg'
                    expected = {'ffmpeg.exe', 'ffprobe.exe', 'codec.dll', 'extra.DLL'}
                    self.assertEqual({p.name for p in folder.iterdir()}, expected)
                    self.assertFalse((folder.parent / 'ffprobe').exists())
                    self.assertEqual(download.call_count, 1)
                    for name in files:
                        self.assertEqual(index[name]['entry'], name + '.exe')
                        self.assertEqual(set(index[name]['files']), expected)
                    bootstrap.install(target)
                    self.assertEqual(download.call_count, 1)
                    (folder / 'codec.dll').write_bytes(b'corrupt')
                    bootstrap.install(target)
                    self.assertEqual(download.call_count, 2)
                    self.assertEqual((folder / 'codec.dll').read_bytes(), b'codec.dll')
                    # Cache may not hide an extra DLL or stale ffplay.
                    (folder / 'ffplay.exe').write_bytes(b'old')
                    bootstrap.install(target)
                    self.assertFalse((folder / 'ffplay.exe').exists())
                    files['ffprobe']['sha256'] = '0' * 64
                    with self.assertRaisesRegex(ValueError, 'pair'):
                        bootstrap.install(target)

    def test_shared_archive_requires_both_executables_and_dll(self):
        for names in (('ffmpeg.exe', 'x.dll'), ('ffmpeg.exe', 'ffprobe.exe')):
            with self.subTest(names=names), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                archive = root / 'shared.zip'
                with zipfile.ZipFile(archive, 'w') as bundle:
                    for name in names:
                        bundle.writestr('build/bin/' + name, b'payload')
                asset = dict(bundle=dict(keep_folder=True, folder_name='build/bin', entry='ffmpeg.exe'))
                with self.assertRaisesRegex(ValueError, 'shared FFmpeg'):
                    bootstrap.unpack(archive, asset, root, 'ffmpeg')

    def test_xz_archives_select_manifest_entries_and_record_file_versions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'ffmpeg.tar.xz'
            with tarfile.open(archive, 'w:xz') as bundle:
                for name in bootstrap.TOOLS:
                    member = tarfile.TarInfo('build/bin/' + name)
                    member.size = len(name)
                    member.mode = 0o755
                    bundle.addfile(member, io.BytesIO(name.encode()))
            files = {name: dict(url=archive.as_uri(), sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),
                               entry='build/bin/' + name, version='platform-release') for name in bootstrap.TOOLS}
            manifest = {'tools': {name: {'version': 'mixed', 'files': {'linux-x86_64': asset}} for name, asset in files.items()}}
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest):
                index = bootstrap.install('x86_64-unknown-linux-gnu')
            self.assertEqual((root / 'src-tauri/binaries/ffprobe-x86_64-unknown-linux-gnu').read_bytes(), b'ffprobe')
            self.assertEqual(index['tools']['ffmpeg']['version'], 'platform-release')

    def test_nonfree_pinned_macos_binaries_are_rejected_before_download(self):
        manifest = json.loads(json.dumps(bootstrap.MANIFEST))
        manifest['tools']['ffmpeg']['files']['darwin-aarch64']['sha256'] = next(iter(bootstrap.NONREDISTRIBUTABLE_SHA256))
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(bootstrap, 'ROOT', Path(tmp)), patch.object(bootstrap, 'MANIFEST', manifest), patch.object(bootstrap, 'checked_download', side_effect=AssertionError('unexpected download')):
                with self.assertRaisesRegex(ValueError, 'nonfree'):
                    bootstrap.install('aarch64-apple-darwin')

    def test_nested_macos_signing_is_inside_out_and_rehashes_local_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            payload = root / 'src-tauri/binaries/tools/yt-dlp'
            framework = payload / '_internal/Python.framework'
            framework.mkdir(parents=True)
            (framework / 'Python').write_bytes(bytes.fromhex('cffaedfe') + b'library')
            (payload / 'yt-dlp').write_bytes(bytes.fromhex('cffaedfe') + b'executable')
            index = payload.parent / 'index.json'
            index.write_text(json.dumps({'tools': {'yt-dlp': {}}}))
            gallery = payload.parent / 'gallery-dl'
            (gallery / '_internal').mkdir(parents=True)
            (gallery / 'gallery-dl').write_bytes(bytes.fromhex('cffaedfe') + b'gallery')
            (gallery / '_internal/Python').write_bytes(bytes.fromhex('cffaedfe') + b'library')
            checksum = root / 'src-tauri/binaries/gallery-dl.sha256'
            checksum.write_text('unsigned-hash\n')
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap.sys, 'platform', 'darwin'), patch.dict(bootstrap.os.environ, {'APPLE_SIGNING_IDENTITY': '-'}), patch.object(bootstrap.subprocess, 'run') as run:
                bootstrap.sign_macos()
            self.assertEqual(run.call_args_list, [], 'ad-hoc builds keep upstream signatures')
            self.assertEqual(checksum.read_text(), 'unsigned-hash\n')

            def sign(command, **_):
                # codesign rewrites the bytes: the recorded hash must be of the signed entry.
                if '--sign' in command:
                    path = Path(command[-1])
                    if path.is_file():
                        path.write_bytes(path.read_bytes() + b'+signed')
                return bootstrap.subprocess.CompletedProcess(command, 0, '', '')
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap.sys, 'platform', 'darwin'), patch.dict(bootstrap.os.environ, {'APPLE_SIGNING_IDENTITY': 'Developer ID Application: Test'}), patch.object(bootstrap.subprocess, 'run', side_effect=sign) as run:
                bootstrap.sign_macos()
            signed = [Path(call.args[0][-1]) for call in run.call_args_list if '--sign' in call.args[0]]
            self.assertEqual(signed[:3], [framework / 'Python', payload / 'yt-dlp', framework])
            self.assertEqual(sorted(signed[3:]), sorted([gallery / '_internal/Python', gallery / 'gallery-dl']))
            self.assertTrue(all('Developer ID Application: Test' in call.args[0] for call in run.call_args_list if '--sign' in call.args[0]))
            self.assertEqual(checksum.read_text(), hashlib.sha256(bytes.fromhex('cffaedfe') + b'gallery+signed').hexdigest() + '\n')
            self.assertIn('_internal/Python.framework/Python', json.loads(index.read_text())['tools']['yt-dlp']['files'])
            self.assertTrue(all(str(root / 'scripts/tools-entitlements.plist') in call.args[0] for call in run.call_args_list if '--sign' in call.args[0]))

            # Self-signed identity (a SHA-1): library validation off, since there is no Team ID. A
            # flattened Python.framework is ambiguous to codesign: its loose copy is signed outside.
            def sign_flattened(command, **kwargs):
                if '--sign' in command and Path(command[-1]).is_relative_to(framework):
                    return bootstrap.subprocess.CompletedProcess(command, 1, '', 'bundle format is ambiguous (could be app or framework)')
                return sign(command, **kwargs)
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap.sys, 'platform', 'darwin'), patch.dict(bootstrap.os.environ, {'APPLE_SIGNING_IDENTITY': 'AB' * 20}), patch.object(bootstrap.subprocess, 'run', side_effect=sign_flattened) as run:
                bootstrap.sign_macos()
            signs = [call.args[0] for call in run.call_args_list if '--sign' in call.args[0]]
            self.assertTrue(all(str(root / 'scripts/tools-entitlements-self-signed.plist') in command for command in signs))
            self.assertTrue((framework / 'Python').read_bytes().endswith(b'+signed+signed'), 'loose copy signed and copied back')

    def test_checksum_mismatch_and_archive_traversal_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'bad.zip'
            with zipfile.ZipFile(source, 'w') as archive:
                archive.writestr('../escaped', b'outside')
            asset = {'url': source.as_uri(), 'sha256': '0' * 64, 'entry': '../escaped'}
            with self.assertRaises(ValueError):
                bootstrap.checked_download(asset, root / 'download')
            asset['sha256'] = hashlib.sha256(source.read_bytes()).hexdigest()
            manifest = {'tools': {name: {'version': '1', 'files': {'linux-x86_64': asset}} for name in bootstrap.TOOLS}}
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest):
                with self.assertRaises(ValueError):
                    bootstrap.install('x86_64-unknown-linux-gnu')
            self.assertFalse((root / 'escaped').exists())
            self.assertFalse((root / 'src-tauri/binaries/tools/index.json').exists())


if __name__ == '__main__':
    unittest.main()
