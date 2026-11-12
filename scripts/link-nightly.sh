#!/usr/bin/env bash
# Python's stdlib handles TSV quoting, process isolation and atomic history updates.
set -euo pipefail
exec python3 - "$(cd -- "$(dirname -- "$0")/.." && pwd)" "$@" <<'PY'
import argparse
import csv
from datetime import datetime, timezone
import fcntl
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
parser = argparse.ArgumentParser(description='Anonymous link matrix + PASS regression diff (no network in --dry-run).')
parser.add_argument('--links', required=True, type=Path)
parser.add_argument('--out', required=True, type=Path)
parser.add_argument('--dry-run', action='store_true')
parser.add_argument('--matrix-bin', type=Path, default=os.environ.get('LINK_MATRIX_BINARY', repo / 'src-tauri/target/debug/examples/link_matrix'))
parser.add_argument('--bin-dir', type=Path, default=os.environ.get('LINK_MATRIX_BIN_DIR'))
parser.add_argument('--resources-dir', type=Path, default=os.environ.get('LINK_MATRIX_RESOURCES_DIR'))
parser.add_argument('--timeout', type=int, default=21600, help='Total matrix deadline in seconds (default 6 hours)')
args = parser.parse_args(sys.argv[2:])
run = None
lock_file = None
lock = args.out.resolve() / '.lock'


def interrupted(signum, _frame):
    raise RuntimeError(f'received signal {signum}')


signal.signal(signal.SIGTERM, interrupted)
signal.signal(signal.SIGHUP, interrupted)


def write_tsv(path, fields, rows):
    temporary = path.with_suffix('.tmp')
    with temporary.open('w', newline='') as file:
        writer = csv.DictWriter(file, fields, delimiter='\t', lineterminator='\n', extrasaction='ignore')
        writer.writeheader()
        writer.writerows(rows)
    temporary.replace(path)


def key(row):
    # Exact URLs, first duplicate wins, matching the matrix parser. Blank fixture
    # gaps have no URL identity, so retain site+label to report them individually.
    return row['url'] or ('gap', row['site'], row['label'])


def read_links(path):
    with path.open(newline='') as file:
        data = list(csv.reader((line for line in file if line.strip() and not line.lstrip().startswith('#')), delimiter='\t'))
    if not data:
        raise ValueError('empty link matrix')
    header = data.pop(0) if 'url' in data[0] else ['site', 'label', 'expected_type', 'url']
    required = {'site', 'label', 'expected_type', 'url'}
    if not required.issubset(header) or len(set(header)) != len(header):
        raise ValueError('links require unique site,label,expected_type,url columns')
    if 'fixture_verdict' in header and not args.dry_run:
        raise ValueError('fixture_verdict is only allowed with --dry-run')
    rows = {}
    for fields in data:
        if len(fields) < len(header):
            raise ValueError('short matrix row')
        row = {name: value.strip() for name, value in zip(header, fields)}
        identity = key(row)
        if identity in rows:
            continue
        verdict = row.get('fixture_verdict', 'PASS')
        if args.dry_run and verdict not in {'PASS', 'FAIL', 'LOGIN', 'GONE'}:
            raise ValueError('invalid fixture_verdict')
        rows[identity] = row
    if not rows:
        raise ValueError('empty link matrix')
    return rows


def read_results(path):
    with path.open(newline='') as file:
        reader = csv.DictReader(file, delimiter='\t')
        if not {'site', 'label', 'expected', 'url', 'verdict', 'login', 'auth_mode'}.issubset(reader.fieldnames or []):
            raise ValueError('results.tsv is missing required columns')
        rows = {}
        for row in reader:
            if None in row or any(value is None for value in row.values()):
                raise ValueError('malformed results.tsv row')
            if row['auth_mode'] != 'anonymous' or row['login'] != 'no':
                raise ValueError('non-anonymous matrix results refused')
            if row['verdict'] not in {'PASS', 'FAIL', 'LOGIN', 'GONE'} or key(row) in rows:
                raise ValueError('invalid or duplicate result row')
            rows[key(row)] = row
    if not rows:
        raise ValueError('empty results.tsv')
    return rows


def notify(status):
    command = os.environ.get('NOTIFY_CMD', '')
    if not command:
        return
    env = dict(os.environ, LINK_NIGHTLY_STATUS=str(status), LINK_NIGHTLY_DIR=str(run or ''),
               LINK_NIGHTLY_RESULTS=str(run / 'results.tsv') if run else '',
               LINK_NIGHTLY_DIFF=str(run / 'diff.tsv') if run else '',
               LINK_NIGHTLY_REGRESSIONS=str(run / 'regressions.tsv') if run else '')
    try:
        process = subprocess.Popen(['/bin/sh', '-c', command], env=env, start_new_session=True)
        try:
            code = process.wait(timeout=60)
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            raise
        if code:
            raise subprocess.CalledProcessError(code, command)
    except (OSError, subprocess.SubprocessError, RuntimeError, KeyboardInterrupt) as error:
        print(f'link-nightly: notification failed: {error}', file=sys.stderr)


def main():
    global lock_file, run
    if args.timeout < 1:
        raise ValueError('--timeout must be positive')
    links = read_links(args.links)
    lock.parent.mkdir(parents=True, exist_ok=True)
    lock_file = lock.open('a')
    try:
        fcntl.flock(lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise ValueError(f'another run holds {lock}') from None
    history = lock.parent / ('dry-run' if args.dry_run else 'anonymous')
    history.mkdir(exist_ok=True)
    previous = sorted(path.parent for path in history.glob('*/complete'))
    stamp = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H%M%S.%fZ')
    run = history / stamp
    run.mkdir()
    # Freeze input for this run; no concurrent edit can change the binary's rows.
    snapshot = run / 'links.tsv'
    write_tsv(snapshot, ['site', 'label', 'expected_type', 'url'], links.values())
    results = run / 'results.tsv'
    if args.dry_run:
        write_tsv(results, ['site', 'label', 'expected', 'url', 'verdict', 'login', 'auth_mode'], [
            dict(row, expected=row['expected_type'], verdict=row.get('fixture_verdict', 'PASS') if row['url'] else 'FAIL',
                 login='no', auth_mode='anonymous') for row in links.values()])
    else:
        if not args.bin_dir or not args.resources_dir:
            raise ValueError('live runs require --bin-dir and --resources-dir (or LINK_MATRIX_*_DIR)')
        binary = args.matrix_bin.resolve(strict=True)
        bins = args.bin_dir.resolve(strict=True)
        resources = args.resources_dir.resolve(strict=True)
        # No credentials, cookies, proxy settings, telemetry keys, or notification
        # command reach the matrix/engines. A fresh HOME also hides browser profiles.
        env = {name: os.environ[name] for name in (
            'PATH', 'DISPLAY', 'XAUTHORITY', 'DBUS_SESSION_BUS_ADDRESS',
            'XDG_RUNTIME_DIR', 'LD_LIBRARY_PATH') if name in os.environ}
        with tempfile.TemporaryDirectory(prefix='nezzel-anonymous-') as home:
            env.update(HOME=home, XDG_CONFIG_HOME=home, XDG_DATA_HOME=home, XDG_CACHE_HOME=home,
                       CI='1', LANG='C.UTF-8', LINK_MATRIX_BIN_DIR=str(bins),
                       LINK_MATRIX_RESOURCES_DIR=str(resources))
            command = [str(binary), '--links', str(snapshot), '--out', str(run), '--jobs', '2']
            with (run / 'matrix.log').open('w') as log:
                process = subprocess.Popen(command, env=env, cwd=home, stdout=log, stderr=log, start_new_session=True)
                try:
                    status = process.wait(timeout=args.timeout)
                except BaseException:
                    # Kill the whole group, including downloader children, on timeout/interruption.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    raise
            if status:
                raise ValueError(f'matrix exited {status}; see {run / "matrix.log"}')
    current = read_results(results)
    if current.keys() != links.keys() or any(
            row['expected'] != links[identity]['expected_type'] or
            row['site'] != links[identity]['site'] or row['label'] != links[identity]['label']
            for identity, row in current.items()):
        raise ValueError('incomplete or mismatched matrix results; baseline unchanged')
    before = read_results(previous[-1] / 'results.tsv') if previous else {}
    if previous and not any(row['verdict'] == 'PASS' for row in before.values()):
        print('link-nightly: warning: previous run had zero PASS rows', file=sys.stderr)
    changes, regressions = [], []
    for identity in list(current) + [identity for identity in before if identity not in current]:
        old, new = before.get(identity), current.get(identity)
        row = new or old
        prior = old['verdict'] if old else 'NEW'
        now = new['verdict'] if new else 'REMOVED'
        changed_expectation = old and new and old['expected'] != new['expected']
        if prior == now and not changed_expectation:
            continue
        change = dict(site=row['site'], label=row['label'], url=row['url'], previous=prior,
                      current=now, change='EXPECTATION' if changed_expectation else 'VERDICT')
        changes.append(change)
        if old and new and not changed_expectation and prior == 'PASS' and now != 'PASS':
            regressions.append(change)
    fields = ['site', 'label', 'url', 'previous', 'current', 'change']
    write_tsv(run / 'diff.tsv', fields, changes)
    write_tsv(run / 'regressions.tsv', fields, regressions)
    (run / 'complete.tmp').write_text(f'previous={previous[-1] if previous else "none"}\n')
    (run / 'complete.tmp').replace(run / 'complete')
    print(f'link-nightly: {run} ({len(current)} rows, {len(regressions)} regressions)')
    if regressions:
        print((run / 'regressions.tsv').read_text(), end='')
        return 1
    return 0


status = 2
try:
    status = main()
except (OSError, ValueError, subprocess.SubprocessError, KeyboardInterrupt, RuntimeError) as error:
    print(f'link-nightly: {error}', file=sys.stderr)
finally:
    if lock_file is not None:
        lock_file.close()
if status:
    notify(status)
sys.exit(status)
PY
