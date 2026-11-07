"""Offline source-release checks, included in the existing test:tools CI gate."""
import json
import contextlib
import hashlib
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('source_bundle', ROOT / 'scripts/source-bundle.py')
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class SourceBundleTests(unittest.TestCase):
    def test_dry_run_reads_current_manifests_without_downloading_or_writing(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'absent'
            run = subprocess.run(['sh', 'scripts/source-bundle.sh', '2.3.4-beta.1', str(output), '--dry-run'],
                                 cwd=ROOT, text=True, capture_output=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            plan = json.loads(run.stdout)
            self.assertEqual(plan['archive'], 'Nezzel-2.3.4-beta.1-SOURCES.tar.gz')
            self.assertEqual(plan['tag'], 'app-v2.3.4-beta.1')
            self.assertFalse(output.exists())
            for name in ('aria2', 'gallery-dl'):
                manifest = json.loads((ROOT / f'scripts/{name}-manifest.json').read_text())
                row = next(row for row in plan['downloads'] if row['component'] == name)
                self.assertEqual(row['url'], manifest['source']['url'])
                self.assertEqual(row['sha256'], manifest['source']['sha256'])
                self.assertEqual(row['version'], manifest['version'])
            tools = json.loads((ROOT / 'src-tauri/src/binaries/manifest.json').read_text())['tools']
            urls = {row['url'] for row in plan['downloads']}
            for asset in tools['ffmpeg']['files'].values():
                version = asset['version']
                if asset['url'].startswith('https://ffmpeg.martin-riedl.de/'):
                    self.assertIn(f'https://ffmpeg.org/releases/ffmpeg-{version}.tar.xz', urls)
                else:
                    self.assertIn('https://github.com/FFmpeg/FFmpeg/archive/' + version.split('-g')[1] + '.tar.gz', urls)
                    tag = asset['url'].split('/download/')[1].split('/')[0]
                    self.assertIn('https://github.com/yt-dlp/FFmpeg-Builds/archive/refs/tags/' + tag + '.tar.gz', urls)
            self.assertIn('README.md', plan['layout'])
            self.assertIn('SHA256SUMS', plan['layout'])
            self.assertIn('licenses/ffmpeg-COPYING.GPLv3', plan['layout'])
            stdout = io.StringIO()
            with patch.object(bundle.urllib.request, 'urlopen', side_effect=AssertionError('dry run used network')), \
                    patch.object(sys, 'argv', ['source-bundle.py', '2.3.4-beta.1', str(output), '--dry-run']), \
                    contextlib.redirect_stdout(stdout):
                bundle.main()
            self.assertEqual(json.loads(stdout.getvalue()), plan)

    def test_download_rejects_checksum_mismatch_and_html_instead_of_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / 'source.tar.gz'
            for row, data in [
                ({'sha256': '0' * 64}, b'wrong source'),
                ({}, b'<html>not a source archive</html>'),
            ]:
                with self.subTest(row=row), patch.object(bundle.urllib.request, 'urlopen', return_value=io.BytesIO(data)):
                    with self.assertRaises(ValueError):
                        bundle.download(dict(url='https://upstream.invalid/source.tar.gz', path='sources/source.tar.gz', **row), destination)
                    self.assertFalse(destination.exists())

    def test_archive_carries_sources_licences_recipes_and_internal_checksums(self):
        source = io.BytesIO()
        with tarfile.open(fileobj=source, mode='w:gz') as tar:
            member = tarfile.TarInfo('upstream/COPYING')
            member.size = len(b'upstream licence')
            tar.addfile(member, io.BytesIO(b'upstream licence'))
        payload = source.getvalue()
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            planned = bundle.plan('2.3.4')
            planned['downloads'] = [dict(component='fixture', version='1.0',
                                         url='https://upstream.invalid/source.tar.gz', path='sources/source.tar.gz',
                                         sha256=hashlib.sha256(payload).hexdigest())]
            with patch.object(bundle.urllib.request, 'urlopen', return_value=io.BytesIO(payload)), \
                    contextlib.redirect_stdout(io.StringIO()):
                bundle.build(planned, output, 'public-owner/nezzel')
            with tarfile.open(output / 'Nezzel-2.3.4-SOURCES.tar.gz') as tar:
                prefix = 'Nezzel-2.3.4-SOURCES/'
                self.assertEqual(tar.extractfile(prefix + 'sources/source.tar.gz').read(), payload)
                readme = tar.extractfile(prefix + 'README.md').read().decode()
                self.assertIn('https://github.com/public-owner/nezzel/tree/app-v2.3.4', readme)
                sums = tar.extractfile(prefix + 'SHA256SUMS').read().decode()
                for path in ('sources/source.tar.gz', 'licenses/aria2-COPYING',
                             'recipes/nezzel/fetch-aria2.py', 'provenance.json'):
                    data = tar.extractfile(prefix + path).read()
                    self.assertIn(hashlib.sha256(data).hexdigest() + '  ' + path + '\n', sums)

    def test_provider_library_versions_resolve_exact_sources_and_report_ambiguous_pins(self):
        sources, missing = bundle.library_sources('ffmpeg version 9.0.2\nconfiguration: --enable-gpl\n'
                                                  'libass     0.17.5\nopus 1.6.1\nx264 0.165.x\nunknownlib 2.0\n')
        self.assertEqual([row['url'] for row in sources], [
            'https://github.com/libass/libass/archive/refs/tags/0.17.5.tar.gz',
            'https://github.com/xiph/opus/archive/refs/tags/v1.6.1.tar.gz'])
        self.assertEqual(missing, ['x264 0.165.x: no exact source revision',
                                   'unknownlib 2.0: no source URL mapping'])

    def test_historical_recipe_and_dependency_resolution_preserves_provider_gaps(self):
        with tempfile.TemporaryDirectory() as tmp:
            stage = Path(tmp)
            history = stage / 'history.json'
            history.write_text(json.dumps([{'sha': 'a' * 40, 'commit': {
                'committer': {'date': '2026-09-19T01:00:00Z'}}}]))
            (stage / 'versions.txt').write_text('libass 0.17.5\nx264 0.165.x\n')
            planned = {'downloads': [
                {'component': 'riedl-recipe-history', 'path': 'history.json', 'before': '2026-09-20T01:00:00Z'},
                {'component': 'riedl-build-info', 'path': 'versions.txt', 'url': 'https://provider.invalid/versions.txt'},
            ], 'layout': []}

            def fetch(row, destination):
                destination.parent.mkdir(parents=True, exist_ok=True)
                if row['component'] == 'libass':
                    raise OSError('upstream unavailable')
                destination.write_bytes(b'historical recipe')

            with patch.object(bundle, 'download', side_effect=fetch) as download, \
                    contextlib.redirect_stderr(io.StringIO()):
                bundle.provider_sources(planned, stage)
            self.assertEqual(download.call_args_list[0].args[0]['url'],
                             bundle.RIEDL_REPO + '/archive/' + 'a' * 40 + '.tar.gz')
            readme = bundle.readme(dict(planned, tag='app-v1.0.0', binaries=[]), 'owner/repo')
            self.assertIn('x264 0.165.x: no exact source revision', readme)
            self.assertIn('upstream unavailable', readme)
            history.write_text(json.dumps([{'sha': 'b' * 40, 'commit': {
                'committer': {'date': '2026-09-21T01:00:00Z'}}}]))
            with self.assertRaisesRegex(ValueError, 'newer than the pinned build'):
                bundle.provider_sources(planned, stage)

    def test_release_checksums_cover_downloaded_assets_and_exclude_old_sums(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / 'latest.json').write_bytes(b'updater')
            (directory / 'Nezzel-1.2.3-SOURCES.tar.gz').write_bytes(b'sources')
            (directory / 'SHA256SUMS').write_text('stale sums')
            bundle.checksums(directory)
            lines = (directory / 'SHA256SUMS').read_text().splitlines()
            self.assertEqual(lines, [hashlib.sha256(b'sources').hexdigest() + '  Nezzel-1.2.3-SOURCES.tar.gz',
                                     hashlib.sha256(b'updater').hexdigest() + '  latest.json'])


if __name__ == '__main__':
    unittest.main()
