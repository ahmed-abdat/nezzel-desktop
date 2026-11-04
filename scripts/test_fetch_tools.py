"""Offline bundle preparation contracts; run by the Rust CI jobs."""
import hashlib
import importlib.util
import json
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
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap.sys, 'platform', 'darwin'), patch.dict(bootstrap.os.environ, {'APPLE_SIGNING_IDENTITY': '-'}), patch.object(bootstrap.subprocess, 'run') as run:
                bootstrap.sign_macos()
            self.assertEqual(run.call_args_list, [], 'ad-hoc builds keep upstream signatures')
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap.sys, 'platform', 'darwin'), patch.dict(bootstrap.os.environ, {'APPLE_SIGNING_IDENTITY': 'Developer ID Application: Test'}), patch.object(bootstrap.subprocess, 'run') as run:
                bootstrap.sign_macos()
            signed = [Path(call.args[0][-1]) for call in run.call_args_list if '--sign' in call.args[0]]
            self.assertEqual(signed, [framework / 'Python', payload / 'yt-dlp', framework])
            self.assertIn('_internal/Python.framework/Python', json.loads(index.read_text())['tools']['yt-dlp']['files'])

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
