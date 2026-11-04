"""Offline image-sidecar bootstrap checks, also run on the Vitest push path."""
import importlib.util
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch
import hashlib
import contextlib
import io

spec = importlib.util.spec_from_file_location('fetch_gallery', Path(__file__).with_name('fetch-gallery-dl.py'))
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class BootstrapTests(unittest.TestCase):
    def test_native_smoke_rejects_version_only_executables_with_missing_extractors(self):
        version = bootstrap.MANIFEST['version'] + '\n'
        broken = subprocess.CalledProcessError(1, ['gallery-dl', '--list-extractors'], stderr="ModuleNotFoundError: No module named 'gallery_dl.extractor.2ch'")
        with patch.object(bootstrap.subprocess, 'check_output', side_effect=[version, broken]):
            with self.assertRaises(subprocess.CalledProcessError):
                bootstrap.smoke(Path('gallery-dl'))
        registry = 'Category: twitter\nCategory: instagram\nCategory: reddit\nCategory: bluesky\nCategory: pinterest\n'
        with patch.object(bootstrap.subprocess, 'check_output', side_effect=[version, registry, '# sidecar-smoke.jpg\n']) as run:
            bootstrap.smoke(Path('gallery-dl'))
            self.assertEqual(run.call_args.args[0][-1], 'https://nezzel.invalid/sidecar-smoke.jpg')
            self.assertIn('-s', run.call_args.args[0])
        with patch.object(bootstrap.subprocess, 'check_output', side_effect=[version, registry, '']):
            with self.assertRaises(ValueError):
                bootstrap.smoke(Path('gallery-dl'))
        with patch.object(bootstrap.subprocess, 'check_output', side_effect=[version, 'Category: instagram\n']):
            with self.assertRaises(ValueError):
                bootstrap.smoke(Path('gallery-dl'))

    def test_rejected_download_cannot_be_installed(self):
        with tempfile.TemporaryDirectory() as tmp:
            source, destination = Path(tmp) / 'source', Path(tmp) / 'binary'
            source.write_bytes(b'wrong bytes')
            with self.assertRaises(ValueError):
                bootstrap.checked_download({'url': source.as_uri(), 'sha256': '0' * 64}, destination)
            self.assertFalse(destination.exists())

    def test_official_asset_cache_is_checked_and_corruption_is_replaced(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'scripts').mkdir()
            (root / 'scripts/gallery-dl-build-requirements.txt').write_text('locked')
            source = root / 'payload'
            source.write_bytes(b'verified official executable')
            manifest = {**bootstrap.MANIFEST, 'assets': {'windows-x64': {'url': source.as_uri(), 'sha256': hashlib.sha256(source.read_bytes()).hexdigest()}}}
            with patch.object(bootstrap, 'ROOT', root), patch.object(bootstrap, 'MANIFEST', manifest), patch.object(bootstrap, 'host_target', return_value='test-host'):
                installed = bootstrap.install('x86_64-pc-windows-msvc')
                installed.write_bytes(b'corrupt cache')
                bootstrap.install('x86_64-pc-windows-msvc')
                self.assertEqual(installed.read_bytes(), source.read_bytes())

    def test_missing_targets_fail_closed_unless_development_stub_is_explicit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'scripts').mkdir()
            (root / 'scripts/gallery-dl-build-requirements.txt').write_text('locked')
            with patch.object(bootstrap, 'ROOT', root):
                with self.assertRaises(ValueError):
                    bootstrap.install('missing-target')
                with contextlib.redirect_stderr(io.StringIO()):
                    stub = bootstrap.install('missing-target', allow_unavailable=True)
                self.assertEqual(stub.read_bytes(), bootstrap.STUB)


if __name__ == '__main__':
    unittest.main()
