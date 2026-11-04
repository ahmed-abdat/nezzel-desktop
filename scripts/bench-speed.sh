#!/usr/bin/env bash
# Sequential live-network benchmark; no browser cookies or app settings are read.
set -euo pipefail
exec python3 - "$@" <<'PY'
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description="Download fixed YouTube 1080p + direct fixtures, sequentially; requires yt-dlp, ffmpeg, aria2c.")
parser.add_argument("--ytdlp", default="yt-dlp")
parser.add_argument("--aria2", default="aria2c", help="Path to the bundled aria2c is supported")
parser.add_argument("--out", type=Path, default=Path("../research/speed-bench.md"))
parser.add_argument("--raw", type=Path, help="Optional JSON evidence including exact commands and errors")
parser.add_argument("--runs", type=int, default=2)
parser.add_argument("--timeout", type=float, default=1800, help="Seconds per download, including extraction and merge")
args = parser.parse_args()
if args.runs < 2 or args.timeout <= 0:
    parser.error("Use at least two runs and a positive timeout")
for tool in (args.ytdlp, args.aria2, "ffmpeg"):
    if not shutil.which(tool):
        parser.error(f"Executable unavailable: {tool}")

def version(command):
    return subprocess.check_output(command, text=True, timeout=15).splitlines()[0]

evidence = {
    "date": datetime.now(timezone.utc).isoformat(),
    "host": platform.platform(),
    "ytdlp": version([args.ytdlp, "--version"]),
    "aria2": version([args.aria2, "--version"]),
    "ffmpeg": version(["ffmpeg", "-version"]),
    "runs": args.runs,
    "timeout_seconds": args.timeout,
    "results": [],
}
fixtures = [
    ("youtube1080", "https://www.youtube.com/watch?v=aqz-KE-bpKQ", ["-f", "bestvideo[height=1080][protocol=https]+bestaudio[protocol=https]"]),
    ("direct", "https://download.blender.org/demo/movies/Cycles_Demoreel_2015.mov", []),
]
# Current PerformanceSettings defaults: concurrent_fragments=4, aria2_enabled=false.
modes = {
    "current": ["--concurrent-fragments", "4"],
    # yt-dlp's "http" protocol key covers HTTPS too; "http,https:" is NOT accepted.
    "aria8": ["--concurrent-fragments", "4", "--downloader", f"http:{Path(shutil.which(args.aria2)).resolve()}", "--downloader-args", "aria2c:-x8 -s8 -k1M"],
}
base = [args.ytdlp, "--ignore-config", "--encoding", "utf-8", "--no-cache-dir", "--no-playlist", "--no-progress", "--no-color", "--verbose", "--socket-timeout", "15", "--retries", "0", "--fragment-retries", "0", "--print", "after_move:BENCH|%(height)s|%(format_id)s|%(protocol)s|%(duration)s|%(filepath)j"]

def atomic_write(path, contents):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(path.name + ".tmp")
    pending.write_text(contents)
    pending.replace(path)

def publish():
    rows = evidence["results"]
    lines = ["# Download speed benchmark — large files", "", f"UTC: {evidence['date']}; host: {evidence['host']}.", f"yt-dlp {evidence['ytdlp']}; {evidence['aria2']}; {evidence['ffmpeg']}.", "", "## Method", "", f"{args.runs} sequential full-download attempts per fixture/mode: current then aria8 in round 1; aria8 then current in round 2. Each run gets an empty directory, deleted afterwards. No cookies, app config, or persistent cache. Wall time includes extraction and merge. MiB/s = final media bytes / 1048576 / wall seconds, only for successful complete downloads. Failed/partial transfers have no throughput. Timeout: {args.timeout:g}s; retries disabled consistently to bound failures. Direct files smaller than 100 MiB are rejected.", "", "Current defaults are N4 with aria2 off. aria8 keeps N4 and adds `--downloader http:<aria2c>` + `--downloader-args 'aria2c:-x8 -s8 -k1M'`; yt-dlp's `http` key covers HTTP and HTTPS, while other protocols retain yt-dlp's downloader. Observed downloaders come from yt-dlp's debug log; any run that silently uses the wrong downloader is rejected. Runtime eligibility: aria8 must improve throughput by at least 20% over current in both rounds on matching format/protocol/size, for the affected fixture. Failed or mismatched comparisons do not qualify.", "", "The suggested Blender Big Buck Bunny MOV URL returned HTTP 404 (its index now lists only ZIP variants). Replacement direct video: Cycles Demoreel 2015 from the same official Blender host; HEAD returned HTTP 200, Content-Length 344434359 bytes (328.478 MiB), Accept-Ranges bytes.", ""]
    for name, url, _ in fixtures:
        lines += [f"- {name}: {url}" + (" (`bestvideo[height=1080][protocol=https]+bestaudio[protocol=https]`, exact 1080p HTTP streams; no lower-resolution fallback)." if name == "youtube1080" else ".")]
    lines += ["", "## Measurements", "", "| Fixture | Round | Mode | Exit/status | Seconds | Final MiB | MiB/s | Format / height | Protocol | Observed downloader(s) |", "|---|---:|---|---|---:|---:|---:|---|---|---|"]
    for row in rows:
        size = f"{row['bytes'] / 1048576:.3f}" if row["status"] == "ok" else "—"
        rate = f"{row['mib_s']:.3f}" if row["mib_s"] is not None else "—"
        lines.append(f"| {row['fixture']} | {row['round']} | {row['mode']} | {row['exit_code']}/{row['status']} | {row['seconds']:.3f} | {size} | {rate} | {row['format']} / {row['height']} | {row['protocol']} | {', '.join(row['downloaders']) or '—'} |")
    lines += ["", "## Decision", ""]
    for name, _, _ in fixtures:
        qualified = []
        for round_number in range(1, args.runs + 1):
            pair = {r["mode"]: r for r in rows if r["fixture"] == name and r["round"] == round_number}
            if all(m in pair and pair[m]["mib_s"] for m in ("current", "aria8")) and all(pair['current'][key] == pair['aria8'][key] for key in ('format', 'height', 'protocol', 'bytes')):
                qualified.append(pair["aria8"]["mib_s"] / pair["current"]["mib_s"])
        passes = len(qualified) == args.runs and all(ratio >= 1.2 for ratio in qualified)
        ratios = ", ".join(f"{(ratio - 1) * 100:+.1f}%" for ratio in qualified) or "no successful paired runs"
        lines.append(f"- {name}: {'qualifies' if passes else 'does not qualify'} for runtime expansion; aria8 throughput versus current: {ratios}; {len(qualified)}/{args.runs} comparable pairs.")
        for mode in modes:
            group = [r for r in rows if r["fixture"] == name and r["mode"] == mode]
            successful = [r for r in group if r["status"] == "ok"]
            if successful:
                lines.append(f"  - {mode}: median {statistics.median(r['seconds'] for r in successful):.3f}s, {statistics.median(r['mib_s'] for r in successful):.3f} MiB/s; {len(successful)}/{len(group)} completed.")
    lines += ["", "This rerun supersedes both the rejected 1.5/4 MiB fixture measurements and an invalid large-file attempt using `http,https:`. Offline parsing of the installed yt-dlp confirmed `http,https:<path>` becomes an unavailable default executable and silently falls back to native HTTP; `http:<path>` selects aria2c. The invalid timings are excluded. End-to-end timing includes extraction/startup/merge and reflects this host and network. Fragment concurrency does not split an ordinary HTTP file. Expansion beyond single direct files requires the paired-run threshold above.", "", "## Run commands and failures", "", "Output paths below were temporary and are already deleted after each run.", ""]
    for row in rows:
        lines += [f"### {row['fixture']} / {row['round']} / {row['mode']}", "", "```sh", shlex.join(row["command"]), "```", ""]
        if row["error"]:
            lines += ["```text", row["error"].replace("```", "'''"), "```", ""]
    atomic_write(args.out, "\n".join(lines) + "\n")
    if args.raw:
        atomic_write(args.raw, json.dumps(evidence, indent=2) + "\n")

publish()
with tempfile.TemporaryDirectory(prefix="nezzel-speed-") as root:
    for name, url, format_args in fixtures:
        for round_number in range(1, args.runs + 1):
            order = list(modes) if round_number % 2 else list(reversed(modes))
            for mode in order:
                destination = Path(root) / f"{name}-{round_number}-{mode}"
                destination.mkdir()
                command = base + format_args + modes[mode] + ["-o", str(destination / "media.%(ext)s"), "--", url]
                started = time.monotonic()
                process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                try:
                    while True:
                        remaining = args.timeout - (time.monotonic() - started)
                        if remaining <= 0:
                            raise subprocess.TimeoutExpired(command, args.timeout)
                        try:
                            stdout, stderr = process.communicate(timeout=min(30, remaining))
                            break
                        except subprocess.TimeoutExpired:
                            partial = sum(p.stat().st_size for p in destination.iterdir() if p.is_file())
                            print(f"{name} round={round_number} {mode}: running {time.monotonic() - started:.0f}s, {partial / 1048576:.1f} MiB on disk (not final throughput)", flush=True)
                    code = process.returncode
                except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        stdout, stderr = process.communicate(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        stdout, stderr = process.communicate()
                    if isinstance(error, KeyboardInterrupt):
                        raise
                    code = 124
                    stderr += f"\nTimeout after {args.timeout:g}s"
                seconds = time.monotonic() - started
                errors = [line for line in stderr.splitlines() if line.startswith("ERROR:")]
                row = dict(fixture=name, round=round_number, mode=mode, exit_code=code, status="failed", seconds=seconds, bytes=0, mib_s=None, format="—", height="—", protocol="—", duration="—", downloaders=re.findall(r"Invoking ([\w+]+) downloader", stderr), error="\n".join(errors) + "\n" + stderr[-1500:] if code else "", command=command)
                if code == 0:
                    markers = [line for line in stdout.splitlines() if line.startswith("BENCH|")]
                    if len(markers) == 1:
                        _, height, format_id, protocol, duration, filepath = markers[0].split("|", 5)
                        file = Path(json.loads(filepath))
                        row.update(format=format_id, height=height, protocol=protocol, duration=duration)
                        minimum_bytes = 100 * 1048576 if name == "direct" else 1
                        expected_downloader = "aria2c" if mode == "aria8" else "http"
                        correct_downloader = row['downloaders'] == [expected_downloader] * len(protocol.split('+'))
                        if file.is_file() and file.stat().st_size >= minimum_bytes and (name != "youtube1080" or height == "1080") and correct_downloader:
                            row.update(status="ok", bytes=file.stat().st_size, mib_s=file.stat().st_size / 1048576 / seconds)
                    if row["status"] != "ok":
                        row["error"] = "Missing complete output, expected 1080p height, direct minimum 100 MiB, or correct observed downloader.\n" + stdout[-1000:]
                evidence["results"].append(row)
                shutil.rmtree(destination)
                publish()
                rate = f"{row['mib_s']:.3f} MiB/s" if row["mib_s"] is not None else "no throughput"
                print(f"{name} round={round_number} {mode}: {row['status']} exit={code} {seconds:.3f}s {rate}", flush=True)
print(f"Report: {args.out}", flush=True)
raise SystemExit(0 if all(r["status"] == "ok" for r in evidence["results"]) else 1)
PY
