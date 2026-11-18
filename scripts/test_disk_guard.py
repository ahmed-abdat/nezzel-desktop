"""Offline checks for the bundled yt-dlp hook (test:tools / Rust CI)."""
import importlib.util
from pathlib import Path
import sys
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import patch


class PostProcessingError(Exception):
    pass


class PostProcessor:
    def __init__(self, downloader=None):
        self._downloader = downloader
        self.hooks = list(downloader.hooks)

    def add_progress_hook(self, hook):
        self.hooks.append(hook)


class DiskGuardTest(unittest.TestCase):
    def setUp(self):
        common = ModuleType('yt_dlp.postprocessor.common')
        common.PostProcessor = PostProcessor
        utils = ModuleType('yt_dlp.utils')
        utils.PostProcessingError = PostProcessingError
        with patch.dict(sys.modules, {'yt_dlp.postprocessor.common': common, 'yt_dlp.utils': utils}):
            spec = importlib.util.spec_from_file_location('nezzel_guard', Path(__file__).parents[1] / 'src-tauri/src/runners/disk_guard.py')
            self.module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(self.module)
        self.module.NEZZEL_ROOT = '/target'
        self.hooks = []
        self.downloader = SimpleNamespace(hooks=self.hooks, _pps={}, add_postprocessor_hook=self.hooks.append)
        self.existing = PostProcessor(self.downloader)
        self.downloader._pps['post_process'] = [self.existing]
        self.guard = self.module.NezzelDiskPP(self.downloader)

    def test_budget_before_transfer(self):
        for size, free, allowed in [(0, 209715199, False), (0, 209715200, True), (101, 111, False), (101, 112, True)]:
            with self.subTest(size=size, free=free), patch.object(self.module.shutil, 'disk_usage', return_value=SimpleNamespace(free=free)):
                if allowed:
                    self.guard.run({'filesize': size})
                else:
                    with self.assertRaisesRegex(PostProcessingError, 'diskFull'):
                        self.guard.run({'filesize': size})

    def test_existing_and_future_processors_receive_guard_before_work(self):
        future = PostProcessor(self.downloader)
        for processor in [self.existing, future]:
            with patch.object(self.module.shutil, 'disk_usage', return_value=SimpleNamespace(free=109)):
                with self.assertRaisesRegex(PostProcessingError, 'diskFull'):
                    for hook in processor.hooks:
                        hook({'status': 'started', 'info_dict': {'requested_formats': [{'filesize': 70}, {'filesize': 30}]}})

    def test_unknown_estimate_uses_actual_file_before_processing(self):
        with patch.object(self.module.shutil, 'disk_usage', return_value=SimpleNamespace(free=109)), patch.object(self.module.os.path, 'isfile', return_value=True), patch.object(self.module.os.path, 'getsize', return_value=100):
            with self.assertRaisesRegex(PostProcessingError, 'diskFull'):
                self.guard.check_processing({'status': 'started', 'info_dict': {'filepath': '/target/file'}})
