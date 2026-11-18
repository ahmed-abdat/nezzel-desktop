"""Bundled Nezzel guard; loaded only by download attempts, never user plugins."""
import os
import shutil
from yt_dlp.postprocessor.common import PostProcessor
from yt_dlp.utils import PostProcessingError


class NezzelDiskPP(PostProcessor):
    def __init__(self, downloader=None):
        super().__init__(downloader)
        # Existing processors captured the hook list at construction; future processors
        # (including the per-video merger) receive it through set_downloader.
        downloader.add_postprocessor_hook(self.check_processing)
        for processors in downloader._pps.values():
            for processor in processors:
                processor.add_progress_hook(self.check_processing)

    def check(self, info, processing=False):
        size = info.get('filesize') or info.get('filesize_approx') or 0
        streams = info.get('requested_formats') or info.get('requested_downloads') or []
        size = max(size, sum(f.get('filesize') or f.get('filesize_approx') or 0 for f in streams))
        if processing:
            paths = {info.get('filepath')}
            paths.update(f.get('filepath') for f in streams)
            size = max(size, sum(os.path.getsize(p) for p in paths if p and os.path.isfile(p)))
        needed = (int(size) * 11 + 9) // 10 if size > 0 else 200 * 1024 * 1024
        try:
            if shutil.disk_usage(NEZZEL_ROOT).free < needed:
                raise PostProcessingError('diskFull')
        except OSError as error:
            raise PostProcessingError('diskSpaceUnavailable') from error

    def check_processing(self, event):
        if event.get('status') == 'started':
            self.check(event.get('info_dict') or {}, processing=True)

    def run(self, info):
        self.check(info)
        return [], info
