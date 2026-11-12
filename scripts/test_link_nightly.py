"""Offline nightly contract checks; included in pnpm run test:tools."""
import ast
import csv
import fcntl
import io
import os
from pathlib import Path
import subprocess
import signal
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('link-nightly.sh')


class LinkNightlyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='nightly space ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.links = self.root / 'links.tsv'
        self.out = self.root / 'history'
        self.write_links()

    def write_links(self, verdicts=('PASS', 'PASS', 'PASS'), order=(0, 1, 2)):
        with self.links.open('w') as file:
            writer = csv.writer(file, delimiter='\t')
            writer.writerow(['site', 'label', 'expected_type', 'url', 'fixture_verdict'])
            for i in order:
                writer.writerow(['example', f'row-{i}', ('video', 'none', 'file')[i],
                                 f'https://example.com/{i}', verdicts[i]])

    def run_nightly(self, *args, env=None):
        return subprocess.run(['bash', str(SCRIPT), '--links', str(self.links),
                               '--out', str(self.out), *args], text=True,
                              capture_output=True, env={**os.environ, 'NOTIFY_CMD': '', **(env or {})})

    def completed(self):
        return sorted((self.out / 'dry-run').glob('*/complete'))

    def test_dry_run_baseline_reorder_and_regression_notify(self):
        first = self.run_nightly('--dry-run')
        self.assertEqual(first.returncode, 0, first.stderr)
        baseline = self.completed()[0].parent
        with (baseline / 'results.tsv').open() as file:
            self.assertEqual(len(list(csv.DictReader(file, delimiter='\t'))), 3)
        self.assertTrue((baseline / 'diff.tsv').is_file())
        self.write_links(order=(2, 0, 1))
        self.assertEqual(self.run_nightly('--dry-run').returncode, 0)
        self.write_links(verdicts=('PASS', 'FAIL', 'PASS'))
        notify = self.root / 'notified'
        result = self.run_nightly('--dry-run', env={
            'NOTIFY_CMD': 'cat "$LINK_NIGHTLY_REGRESSIONS" > "$NOTICE_FILE"',
            'NOTICE_FILE': str(notify)})
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('PASS\tFAIL', result.stdout)
        self.assertIn('https://example.com/1', notify.read_text())
        self.assertEqual(len(self.completed()), 3)

    def test_removed_inputs_are_diffed_but_are_not_regressions(self):
        self.assertEqual(self.run_nightly('--dry-run').returncode, 0)
        self.write_links(order=(0, 2))
        result = self.run_nightly('--dry-run')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('REMOVED', (self.completed()[-1].parent / 'diff.tsv').read_text())

    def test_bad_fixture_and_overlap_fail_without_baseline(self):
        self.write_links(verdicts=('PASS', 'BOGUS', 'PASS'))
        self.assertEqual(self.run_nightly('--dry-run').returncode, 2)
        self.write_links()
        self.out.mkdir(parents=True, exist_ok=True)
        with (self.out / '.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = self.run_nightly('--dry-run')
            self.assertEqual(result.returncode, 2)
            self.assertFalse(self.completed())
        self.assertEqual(self.run_nightly('--dry-run').returncode, 0)
        self.assertTrue((self.out / '.lock').is_file())

    def test_notify_failure_preserves_regression_status_and_artifacts(self):
        self.assertEqual(self.run_nightly('--dry-run').returncode, 0)
        self.write_links(verdicts=('FAIL', 'PASS', 'PASS'))
        result = self.run_nightly('--dry-run', env={'NOTIFY_CMD': 'exit 9'})
        self.assertEqual(result.returncode, 1)
        self.assertIn('notification failed', result.stderr)
        self.assertEqual(len(self.completed()), 2)

    def fake_matrix(self, mode):
        # A process-boundary fixture: no extractor/network. Tests exercise the real
        # wrapper's environment, result validation, and baseline publication.
        binary = self.root / 'matrix'
        binary.write_text('''#!/usr/bin/env python3
import csv, os, pathlib, sys
assert os.environ['CI'] == '1'
assert 'NIGHTLY_TEST_SECRET' not in os.environ
assert 'NOTIFY_CMD' not in os.environ
assert not pathlib.Path(os.environ['HOME'], 'owner-cookie').exists()
out = pathlib.Path(sys.argv[sys.argv.index('--out') + 1])
links = pathlib.Path(sys.argv[sys.argv.index('--links') + 1])
with links.open() as f:
    rows = list(csv.DictReader(f, delimiter='\\t'))
with (out / 'results.tsv').open('w') as f:
    writer = csv.writer(f, delimiter='\\t')
    writer.writerow(['site', 'label', 'expected', 'url', 'verdict', 'login', 'auth_mode'])
    for row in rows:
        writer.writerow([row['site'], row['label'], row['expected_type'], row['url'],
                         'PASS', 'no', 'anonymous'])
''')
        if mode == 'partial':
            binary.write_text(binary.read_text().replace('for row in rows:', 'for row in rows[:1]:'))
        elif mode == 'auth':
            binary.write_text(binary.read_text().replace("'anonymous'", "'cookies-file'"))
        elif mode == 'crash':
            binary.write_text(binary.read_text() + '\nsys.exit(7)\n')
        elif mode in ('term', 'hup', 'kill'):
            signum = {'term': 'SIGTERM', 'hup': 'SIGHUP', 'kill': 'SIGKILL'}[mode]
            binary.write_text(binary.read_text() + '''
import signal
pathlib.Path(__file__).with_suffix('.pid').write_text(str(os.getpid()))
os.kill(os.getppid(), signal.SIGNAL_NAME)
AFTER_SIGNAL
'''.replace('SIGNAL_NAME', signum).replace('AFTER_SIGNAL', 'sys.exit(0)' if mode == 'kill' else 'signal.pause()'))
        binary.chmod(0o755)
        return str(binary)

    def live_fixture(self, mode):
        self.links.write_text(self.links.read_text().replace('\tfixture_verdict', '')
                              .replace('\tPASS', ''))
        return self.run_nightly('--matrix-bin', self.fake_matrix(mode), env={
            'NIGHTLY_TEST_SECRET': 'not-for-child', 'LINK_MATRIX_BIN_DIR': str(self.root),
            'LINK_MATRIX_RESOURCES_DIR': str(self.root)})

    def test_live_process_receives_isolated_environment(self):
        result = self.live_fixture('valid')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(list((self.out / 'anonymous').glob('*/complete'))), 1)
        self.assertFalse(self.completed())

    def test_partial_authenticated_and_crashed_outputs_never_become_baselines(self):
        for mode in ('partial', 'auth', 'crash'):
            with self.subTest(mode=mode):
                result = self.live_fixture(mode)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(list((self.out / 'anonymous').glob('*/complete')))

    def test_live_mode_rejects_dry_run_verdicts(self):
        result = self.run_nightly('--matrix-bin', self.fake_matrix('valid'))
        self.assertEqual(result.returncode, 2)
        self.assertIn('fixture_verdict', result.stderr)

    def test_termination_signals_stop_matrix_without_publishing_and_release_lock(self):
        for mode in ('term', 'hup'):
            with self.subTest(mode=mode):
                result = self.live_fixture(mode)
                pid = int((self.root / 'matrix.pid').read_text())
                try:
                    self.assertEqual(result.returncode, 2, result.stderr)
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
                    self.assertFalse(list((self.out / 'anonymous').glob('*/complete')))
                    with (self.out / '.lock').open('a') as lock:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    try:
                        os.kill(pid, 9)
                    except ProcessLookupError:
                        pass

    def test_sigkill_releases_lock_for_the_next_run(self):
        result = self.live_fixture('kill')
        self.assertEqual(result.returncode, -signal.SIGKILL, result.stderr)
        self.assertFalse(list((self.out / 'anonymous').glob('*/complete')))
        result = self.live_fixture('valid')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.out / '.lock').is_file())

    def test_notification_timeout_kills_the_entire_process_group(self):
        # Exercise the real hook; force the deadline after both real processes
        # hold the FIFO open. EOF proves descendants were killed too, without sleeps.
        source = SCRIPT.read_text().split("<<'PY'\n", 1)[1].rsplit('\nPY', 1)[0]
        function = next(node for node in ast.parse(source).body
                        if isinstance(node, ast.FunctionDef) and node.name == 'notify')
        namespace = dict(os=os, sys=__import__('sys'), signal=signal, subprocess=subprocess, run=self.root)
        exec(compile(ast.Module(body=[function], type_ignores=[]), str(SCRIPT), 'exec'), namespace)
        fifo = self.root / 'hook.fifo'
        os.mkfifo(fifo)
        helper = self.root / 'hook.py'
        helper.write_text("import os, signal, sys\n"
                          "fd = os.open(sys.argv[1], os.O_WRONLY)\n"
                          "child = os.fork()\n"
                          "if child: os.write(fd, b'R')\n"
                          "signal.pause()\n")
        command = 'exec python3 "$HOOK_HELPER" "$HOOK_FIFO"'
        spawn = subprocess.Popen
        processes = []
        reader = None

        def launch(*args, **kwargs):
            process = spawn(*args, **kwargs)
            processes.append(process)
            real_wait = process.wait

            def wait(timeout=None):
                nonlocal reader
                if timeout is not None:
                    reader = fifo.open('rb', buffering=0)
                    self.assertEqual(reader.read(1), b'R')
                    raise subprocess.TimeoutExpired(command, timeout)
                return real_wait()

            process.wait = wait
            return process

        try:
            with patch.dict(os.environ, NOTIFY_CMD=command, HOOK_HELPER=str(helper), HOOK_FIFO=str(fifo)):
                with patch.object(subprocess, 'Popen', launch), patch('sys.stderr', new_callable=io.StringIO) as errors:
                    namespace['notify'](1)
                self.assertIn('notification failed', errors.getvalue())
            self.assertIsNotNone(processes[0].returncode)
            self.assertEqual(reader.read(1), b'')
        finally:
            for process in processes:
                try:
                    if os.getpgid(process.pid) == process.pid:
                        os.killpg(process.pid, signal.SIGKILL)
                    else:
                        os.kill(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=None)
            if reader is not None:
                reader.close()


if __name__ == '__main__':
    unittest.main()
