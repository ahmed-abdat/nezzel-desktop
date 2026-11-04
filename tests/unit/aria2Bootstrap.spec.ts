import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vitest';

describe('optional aria2 bootstrap', () => {
  it('degrades unsupported targets, missing toolchains, and rejected payloads to disabled stubs', () => {
    const result = spawnSync('python3', ['-B', 'scripts/test_fetch_aria2.py'], { encoding: 'utf8' });
    expect(result.status, result.stderr).toBe(0);
  });
});

describe('gallery-dl bootstrap', () => {
  it('rejects checksum failures, replaces corrupt cache, and keeps development stubs explicit', () => {
    const result = spawnSync('python3', ['-B', 'scripts/test_fetch_gallery.py'], { encoding: 'utf8' });
    expect(result.status, result.stderr).toBe(0);
  });
});

describe('installer tool bootstrap', () => {
  it('verifies pinned archives, preserves onedir dependencies, and repairs corrupt cache offline', () => {
    const result = spawnSync('python3', ['-B', 'scripts/test_fetch_tools.py'], { encoding: 'utf8' });
    expect(result.status, result.stderr).toBe(0);
  });
});
