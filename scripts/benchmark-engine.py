#!/usr/bin/env python3
"""Sequential, bounded CLI measurements. Never reads browser cookies or changes app config."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time

DIRECT = [
    ('sintel', 'https://media.w3.org/2010/05/sintel/trailer.mp4'),
    ('bunny', 'https://media.w3.org/2010/05/bunny/trailer.mp4'),
    ('flower', 'https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4'),
]
HLS = ('hls', 'https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8')
YOUTUBE = 'https://www.youtube.com/watch?v=YsKxkP_kGbE'
LOGIN = 'https://www.reddit.com/r/nasa/comments/1wtjy4r/nasas_view_program_running_on_an_univac_1108_to/'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--aria2', type=Path, required=True)
    parser.add_argument('--ytdlp', default=shutil.which('yt-dlp'))
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    rows = []
    with tempfile.TemporaryDirectory(prefix='nezzel-bench-') as tmp:
        tmp = Path(tmp)
        bin_dir = tmp / 'bin'
        bin_dir.mkdir()
        shutil.copyfile(args.aria2, bin_dir / 'aria2c')
        (bin_dir / 'aria2c').chmod(0o755)
        env = os.environ.copy()
        env['PATH'] = str(bin_dir) + os.pathsep + env['PATH']
        base = [args.ytdlp, '--ignore-config', '--no-cache-dir', '--no-progress', '--no-warnings', '--no-playlist', '--socket-timeout', '12', '--retries', '0', '--fragment-retries', '0']
        version = subprocess.check_output([args.ytdlp, '--version'], text=True).strip()
        def run(name, url, mode, extra, count=3):
            for i in range(count):
                destination = tmp / f'{name}-{mode}-{i}.%(ext)s'
                start = time.monotonic()
                command = base + ['-o', str(destination)] + extra + ['--', url]
                try:
                    result = subprocess.run(command, capture_output=True, text=True, env=env, timeout=120)
                    code, stderr = result.returncode, result.stderr
                except subprocess.TimeoutExpired:
                    code, stderr = 124, 'timeout after 120s'
                seconds = round(time.monotonic() - start, 3)
                sizes = sum(f.stat().st_size for f in tmp.glob(f'{name}-{mode}-{i}.*') if f.is_file())
                run_index = 1 + sum(r['link'] == name and r['mode'] == mode for r in rows)
                row = dict(link=name, url=url, mode=mode, run=run_index, seconds=seconds, code=code, bytes=sizes, error=stderr[-1200:] if code else '')
                rows.append(row)
                args.out.write_text(json.dumps(dict(ytdlp=version, results=rows), indent=2) + '\n')
                print(name, mode, i + 1, seconds, code, sizes, flush=True)
                for f in tmp.glob(f'{name}-{mode}-{i}.*'):
                    if f.is_file():
                        f.unlink()
        for name, url in DIRECT + [HLS]:
            # Interleave modes to reduce drift from CDN/cache/network changes.
            for _ in range(3):
                run(name, url, 'native', ['-f', 'worst', '--concurrent-fragments', '4', '--downloader', 'native'], 1)
                run(name, url, 'aria8', ['-f', 'worst', '--concurrent-fragments', '4', '--downloader', 'aria2c', '--downloader-args', 'aria2c:-x8 -s8 -k1M'], 1)
                run(name, url, 'aria16', ['-f', 'worst', '--concurrent-fragments', '4', '--downloader', 'aria2c', '--downloader-args', 'aria2c:-x16 -s16 -k1M'], 1)
        for fragments in (1, 4):
            run('youtube1080', YOUTUBE, f'fragments{fragments}', ['-f', 'bestvideo[height=1080]+bestaudio', '--concurrent-fragments', str(fragments)], 1)
            run('direct-before-after', DIRECT[0][1], f'fragments{fragments}', ['--concurrent-fragments', str(fragments)], 1)
            run('login-refusal', LOGIN, f'fragments{fragments}', ['-J', '--skip-download', '--concurrent-fragments', str(fragments)], 1)
        for name, _ in DIRECT + [HLS]:
            for mode in ('native', 'aria8', 'aria16'):
                group = [r for r in rows if r['link'] == name and r['mode'] == mode]
                print('MEDIAN', name, mode, statistics.median(r['seconds'] for r in group), 'failures', sum(r['code'] != 0 for r in group))


if __name__ == '__main__':
    main()
