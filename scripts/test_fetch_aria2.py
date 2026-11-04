"""Offline optional-sidecar bootstrap checks, invoked by the Vitest push suite."""
import contextlib
import importlib.util
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('fetch_aria2', Path(__file__).with_name('fetch-aria2.py'))
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class BootstrapTests(unittest.TestCase):
    def test_unavailable_targets_and_failed_builds_produce_a_disabled_stub(self):
        for target, failure in [
            ('aarch64-pc-windows-msvc', None),
            ('x86_64-apple-darwin', None),
            ('aarch64-apple-darwin', subprocess.CalledProcessError(1, 'configure')),
            ('x86_64-pc-windows-msvc', OSError('offline')),
        ]:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                stderr = io.StringIO()
                # Existing source commands are never executed or downloaded in these tests.
                with patch.object(bootstrap, 'ROOT', root), patch.object(sys, 'argv', ['fetch-aria2.py', '--target', target]), patch.object(subprocess, 'check_output', return_value='host: aarch64-apple-darwin\n'), patch.object(bootstrap, 'checked_download', side_effect=failure or AssertionError('unexpected download')), contextlib.redirect_stderr(stderr):
                    try:
                        bootstrap.main()
                        code = 0
                    except Exception:
                        code = 1
                self.assertEqual(code, 0, stderr.getvalue())
                suffix = '.exe' if 'windows' in target else ''
                sidecar = root / 'src-tauri/binaries' / ('aria2c-' + target + suffix)
                self.assertTrue(sidecar.read_bytes().startswith(b'NEZZEL_ARIA2_UNAVAILABLE\n'))
                self.assertIn('warning', stderr.getvalue().lower())

    def test_configure_failure_after_verified_source_does_not_block_build(self):
        import hashlib
        import tarfile
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / ('aria2-' + bootstrap.MANIFEST['version'])
            source.mkdir()
            archive = root / 'source.tar.xz'
            with tarfile.open(archive, 'w:xz') as tar:
                tar.add(source, arcname=source.name)
            manifest = {**bootstrap.MANIFEST, 'source': {'sha256': hashlib.sha256(archive.read_bytes()).hexdigest()}}
            stderr = io.StringIO()
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest), patch.object(sys, 'argv', ['fetch-aria2.py', '--target', 'aarch64-apple-darwin', '--source-archive', str(archive)]), patch.object(subprocess, 'check_output', return_value='host: aarch64-apple-darwin\n'), patch.object(subprocess, 'run', side_effect=subprocess.CalledProcessError(1, './configure')) as configure, contextlib.redirect_stderr(stderr):
                bootstrap.main()
            self.assertTrue(configure.called)
            self.assertTrue((root / 'src-tauri/binaries/aria2c-aarch64-apple-darwin').read_bytes().startswith(b'NEZZEL_ARIA2_UNAVAILABLE\n'))
            self.assertIn('warning', stderr.getvalue().lower())

    def test_checksum_mismatch_never_installs_downloaded_executable(self):
        import hashlib
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'payload.zip'
            source.write_bytes(b'untrusted payload')
            bad_manifest = {**bootstrap.MANIFEST, 'win64': {'url': source.as_uri(), 'sha256': hashlib.sha256(b'expected').hexdigest()}}
            stderr = io.StringIO()
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', bad_manifest), patch.object(sys, 'argv', ['fetch-aria2.py', '--target', 'x86_64-pc-windows-msvc']), patch.object(subprocess, 'check_output', return_value='host: aarch64-apple-darwin\n'), contextlib.redirect_stderr(stderr):
                try:
                    bootstrap.main()
                    code = 0
                except Exception:
                    code = 1
            self.assertEqual(code, 0, stderr.getvalue())
            payload = (root / 'src-tauri/binaries/aria2c-x86_64-pc-windows-msvc.exe').read_bytes()
            self.assertTrue(payload.startswith(b'NEZZEL_ARIA2_UNAVAILABLE\n'))
            self.assertNotEqual(payload, b'untrusted payload')


if __name__ == '__main__':
    unittest.main()
