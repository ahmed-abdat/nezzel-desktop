#!/usr/bin/env node
import { createHash, createPrivateKey, sign, verify, createPublicKey } from 'node:crypto';
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { createWriteStream } from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const json = value => `${JSON.stringify(value, null, 2)}\n`;

async function request(url, fetcher = fetch, timeout = 300_000) {
  if (new URL(url).protocol !== 'https:') throw new Error('Tool sources must use HTTPS');
  const headers = { 'User-Agent': 'Nezzel-manifest/1.0 (+https://github.com/hululsofthq/nezzel-desktop)' };
  // Never send the repository token to a tool host/CDN.
  if (new URL(url).origin === 'https://api.github.com' && process.env.GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  }
  const response = await fetcher(url, { headers, signal: AbortSignal.timeout(timeout) });
  if (!response.ok) throw new Error(`Fetch failed (${response.status}): ${url}`);
  if (response.url && new URL(response.url).protocol !== 'https:') throw new Error('Insecure redirect');
  return response;
}

export async function hashAsset(url, fetcher = fetch) {
  const response = await request(url, fetcher);
  if (!response.body) throw new Error(`Missing download body: ${url}`);
  const hash = createHash('sha256');
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    hash.update(chunk);
  }
  if (!size) throw new Error(`Empty download: ${url}`);
  return hash.digest('hex');
}

async function hashed(url, hashes, fetcher) {
  if (!hashes.has(url)) {
    if (fetcher === fetch) console.error(`Hashing ${url}`);
    hashes.set(url, await hashAsset(url, fetcher));
  }
  return hashes.get(url);
}

async function latest(repo, origin = 'https://api.github.com', fetcher = fetch) {
  const release = await (await request(`${origin}/repos/${repo}/releases/latest`, fetcher)).json();
  if (!release.tag_name || release.draft || release.prerelease || !Array.isArray(release.assets)) {
    throw new Error(`Invalid stable release: ${repo}`);
  }
  return release;
}

function asset(release, name) {
  const value = release.assets.find(item => item.name === name)?.browser_download_url;
  if (!value) throw new Error(`Missing official release asset: ${name}`);
  return value;
}

export async function updateGithubTool(tool, release, hashes, fetcher = fetch) {
  for (const file of Object.values(tool.files)) {
    const name = new URL(file.url).pathname.split('/').pop();
    file.url = asset(release, name);
    file.sha256 = await hashed(file.url, hashes, fetcher);
    if (file.version !== undefined) file.version = release.tag_name;
  }
  tool.version = release.tag_name;
}

export function signManifest(bytes, encodedKey, publicKeyHex) {
  // Errors must never echo the secret or OpenSSL's input buffers.
  try {
    if (!encodedKey) throw new Error();
    const key = createPrivateKey({ key: Buffer.from(encodedKey.trim(), 'base64'), format: 'der', type: 'pkcs8' });
    if (key.asymmetricKeyType !== 'ed25519') throw new Error();
    const signature = sign(null, bytes, key);
    if (!verify(null, bytes, createPublicKey(key), signature)) throw new Error();
    if (publicKeyHex) {
      const hex = publicKeyHex.trim();
      if (!/^[a-fA-F0-9]{64}$/.test(hex)) throw new Error();
      const publicKey = createPublicKey({ key: Buffer.concat([
        Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(hex, 'hex'),
      ]), format: 'der', type: 'spki' });
      if (!verify(null, bytes, publicKey, signature)) throw new Error();
    }
    return signature.toString('base64');
  } catch {
    throw new Error('Invalid Ed25519 PKCS#8 signing key or mismatched NEZZEL_MANIFEST_PUBKEY');
  }
}

export async function bump(manifest, gallery, aria2, fetcher = fetch) {
  // Each tool commits only after all of its assets were hashed; failures retain its pin.
  [manifest, gallery, aria2] = structuredClone([manifest, gallery, aria2]);
  const hashes = new Map();
  let succeeded = 0;
  async function refresh(name, previous, update) {
    const candidate = structuredClone(previous);
    try {
      await update(candidate);
      // Rollback is an explicit operator decision, not inherited by automatic refreshes.
      delete candidate.rollback;
      succeeded++;
      return candidate;
    } catch (error) {
      console.error(`Keeping previous ${name} pin: ${error.message}`);
      return previous;
    }
  }
  manifest.tools['yt-dlp'] = await refresh('yt-dlp', manifest.tools['yt-dlp'], async tool =>
    updateGithubTool(tool, await latest('yt-dlp/yt-dlp', undefined, fetcher), hashes, fetcher));
  manifest.tools.deno = await refresh('deno', manifest.tools.deno, async tool =>
    updateGithubTool(tool, await latest('denoland/deno', undefined, fetcher), hashes, fetcher));

  // FFmpeg archives expire upstream. Keep reviewed, mirrored pins verbatim; upgrades
  // are explicit manifest edits followed by mirror-ffmpeg, never a daily refresh.
  // AtomicParsley stays pinned; it is not part of the daily update lane.
  gallery = await refresh('gallery-dl', gallery, async (gallery) => {
    const gal = await latest('mikf/gallery-dl', 'https://codeberg.org/api/v1', fetcher);
    gallery.version = gal.tag_name.replace(/^v/, '');
    gallery.source.url = asset(gal, `gallery_dl-${gallery.version}.tar.gz`);
    gallery.source.sha256 = await hashed(gallery.source.url, hashes, fetcher);
    for (const file of Object.values(gallery.assets)) {
      file.url = asset(gal, new URL(file.url).pathname.split('/').pop());
      file.sha256 = await hashed(file.url, hashes, fetcher);
    }
  });
  aria2 = await refresh('aria2', aria2, async (aria2) => {
    const aria = await latest('aria2/aria2', undefined, fetcher);
    aria2.version = aria.tag_name.replace(/^release-/, '');
    for (const [field, filename] of [['source', `aria2-${aria2.version}.tar.xz`],
      ['win64', `aria2-${aria2.version}-win-64bit-build1.zip`]]) {
      aria2[field].url = asset(aria, filename);
      aria2[field].sha256 = await hashed(aria2[field].url, hashes, fetcher);
    }
  });
  if (!succeeded) throw new Error('Every tool source failed; nothing to sign');
  manifest.generatedAt = new Date().toISOString();
  return { 'manifest.json': manifest, 'gallery-dl-manifest.json': gallery, 'aria2-manifest.json': aria2 };
}

const UPSTREAM_FFMPEG = /^https:\/\/github\.com\/yt-dlp\/FFmpeg-Builds\/releases\/download\/(autobuild-[^/]+)\/([^/]+)$/;
const MIRROR_REPO = 'hululsofthq/nezzel-desktop';

// Pure: upstream FFmpeg-Builds asset URL -> where our mirror release serves it.
export function mirrorUrl(url, repo = MIRROR_REPO) {
  const match = UPSTREAM_FFMPEG.exec(url);
  if (!match) throw new Error(`Not a yt-dlp/FFmpeg-Builds autobuild asset URL: ${url}`);
  const [, upstreamTag, name] = match;
  const tag = `tools-ffmpeg-${upstreamTag}`;
  return { upstreamTag, name, tag, url: `https://github.com/${repo}/releases/download/${tag}/${name}` };
}

// Pure: the unique archives to mirror (ffmpeg and ffprobe share them) and the manifest with URLs rewritten.
// The upstream URL stays in `source` (the Rust manifest parser ignores unknown fields).
export function planMirror(manifest, repo = MIRROR_REPO) {
  const next = structuredClone(manifest);
  const archives = new Map();
  for (const [toolName, tool] of Object.entries(next.tools)) {
    if (toolName !== 'ffmpeg' && toolName !== 'ffprobe') continue;
    for (const file of Object.values(tool.files)) {
      if (!UPSTREAM_FFMPEG.test(file.url)) continue;
      const target = mirrorUrl(file.url, repo);
      const known = archives.get(file.url);
      if (known && known.sha256 !== file.sha256) throw new Error(`Conflicting sha256 for ${file.url}`);
      archives.set(file.url, { ...target, upstream: file.url, sha256: file.sha256,
        users: [...(known?.users ?? []), `${toolName}`] });
      file.source = file.url;
      file.url = target.url;
    }
  }
  return { manifest: next, archives: [...archives.values()] };
}

function gh(args, options = {}) {
  const result = spawnSync('gh', args, { encoding: 'utf8', ...options });
  if (result.error) throw result.error;
  return result;
}

async function download(url, path) {
  const response = await request(url, fetch, 1_800_000);
  if (!response.body) throw new Error(`Missing download body: ${url}`);
  const hash = createHash('sha256');
  let size = 0;
  const body = Readable.fromWeb(response.body);
  body.on('data', (chunk) => {
    size += chunk.length;
    hash.update(chunk);
  });
  await pipeline(body, createWriteStream(path));
  return { size, sha256: hash.digest('hex') };
}

async function mirrorFfmpeg(dryRun) {
  const manifestPath = join(root, 'src-tauri/src/binaries/manifest.json');
  const { manifest, archives } = planMirror(JSON.parse(await readFile(manifestPath, 'utf8')));
  if (!archives.length) {
    console.log('No upstream FFmpeg-Builds URLs left in the manifest; nothing to mirror.');
    return;
  }
  const temp = await mkdtemp(join(tmpdir(), 'nezzel-ffmpeg-mirror-'));
  try {
    const tags = [...new Set(archives.map(item => item.tag))];
    const existing = new Map(); // tag -> Map(name -> size)
    for (const tag of tags) {
      const view = gh(['release', 'view', tag, '--repo', MIRROR_REPO, '--json', 'assets']);
      // Only "not found" means the release is missing; auth, API or network failures must stop the run.
      if (view.status !== 0 && !/release not found/i.test(view.stderr ?? '')) {
        throw new Error(`release view failed for ${tag}: ${view.stderr}`);
      }
      existing.set(tag, view.status === 0
        ? new Map(JSON.parse(view.stdout).assets.map(a => [a.name, a.size]))
        : null);
    }
    let uploads = 0;
    for (const item of archives) {
      const path = join(temp, item.name);
      let size;
      if (dryRun) {
        // Dry run: confirm the upstream asset still exists and read its size; the real run downloads and checks sha256.
        const head = await fetch(item.upstream, { method: 'HEAD', redirect: 'follow', signal: AbortSignal.timeout(60_000) });
        if (!head.ok) throw new Error(`Upstream asset gone (${head.status}): ${item.upstream}`);
        size = Number(head.headers.get('content-length'));
      } else {
        console.error(`Downloading ${item.upstream}`);
        const got = await download(item.upstream, path);
        if (got.sha256 !== item.sha256) throw new Error(`sha256 mismatch for ${item.name}: got ${got.sha256}, manifest ${item.sha256}`);
        size = got.size;
      }
      const assets = existing.get(item.tag);
      const same = assets?.get(item.name) === size;
      console.log(`${same ? 'skip (same size)' : 'upload'}  ${item.name}  ${size} bytes  [${item.users.join('+')}]`);
      console.log(`  ${item.upstream}\n  -> ${item.url}`);
      if (dryRun || same) {
        await rm(path, { force: true });
        continue;
      }
      if (!assets && !existing.get(item.tag)) {
        const notes = `Mirror of yt-dlp/FFmpeg-Builds \`${item.upstreamTag}\` (https://github.com/yt-dlp/FFmpeg-Builds/releases/tag/${item.upstreamTag}), kept because upstream deletes autobuilds after about two weeks. `
          + 'Archives are byte-identical to upstream (sha256 checked against the app manifest). FFmpeg is GPLv3 in these builds; source and licence: https://github.com/yt-dlp/FFmpeg-Builds and https://ffmpeg.org.';
        const made = gh(['release', 'create', item.tag, '--repo', MIRROR_REPO, '--prerelease', '--title', `FFmpeg builds ${item.upstreamTag} (mirror)`, '--notes', notes]);
        if (made.status !== 0) throw new Error(`release create failed: ${made.stderr}`);
        existing.set(item.tag, new Map());
      }
      const up = gh(['release', 'upload', item.tag, path, '--repo', MIRROR_REPO, '--clobber']);
      if (up.status !== 0) throw new Error(`upload failed for ${item.name}: ${up.stderr}`);
      uploads++;
      await rm(path, { force: true });
    }
    console.log(`\n${archives.length} archives (${archives.map(a => a.name).join(', ')})`);
    const rewritten = Object.entries(manifest.tools).filter(([n]) => n === 'ffmpeg' || n === 'ffprobe')
      .flatMap(([n, t]) => Object.entries(t.files).map(([p, f]) => ({ n, p, url: f.url })));
    console.log(`Manifest entries rewritten (${rewritten.filter(r => r.url.startsWith(`https://github.com/${MIRROR_REPO}/`)).length}):`);
    for (const r of rewritten) console.log(`  ${r.n} ${r.p}: ${r.url}`);
    if (dryRun) {
      console.log('\nDry run: nothing uploaded, manifest.json untouched.');
      return;
    }
    await writeFile(manifestPath, json(manifest));
    console.log(`\nUploaded ${uploads}; manifest.json rewritten. Review, commit and re-run to confirm all skip.`);
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
}

async function main() {
  const args = process.argv.slice(2);
  const mirror = args[0] === 'mirror-ffmpeg';
  const flags = mirror ? args.slice(1) : args;
  if (flags.some(arg => arg !== '--dry-run')) throw new Error('Usage: node scripts/manifest-bump.mjs [mirror-ffmpeg] [--dry-run]');
  const dryRun = flags.includes('--dry-run');
  if (mirror) return mirrorFfmpeg(dryRun);
  if (!dryRun && !process.env.NEZZEL_MANIFEST_SIGNING_KEY) throw new Error('NEZZEL_MANIFEST_SIGNING_KEY is required');
  const paths = { 'manifest.json': 'src-tauri/src/binaries/manifest.json',
    'gallery-dl-manifest.json': 'scripts/gallery-dl-manifest.json', 'aria2-manifest.json': 'scripts/aria2-manifest.json' };
  const current = Object.fromEntries(await Promise.all(Object.entries(paths).map(async ([name, path]) =>
    [name, JSON.parse(await readFile(join(root, path), 'utf8'))])));
  const candidate = await bump(current['manifest.json'], current['gallery-dl-manifest.json'], current['aria2-manifest.json']);
  const temp = await mkdtemp(join(tmpdir(), 'nezzel-manifest-diff-'));
  try {
    for (const [name, value] of Object.entries(candidate)) {
      const path = join(temp, name);
      await writeFile(path, json(value));
      const result = spawnSync('diff', ['-u', join(root, paths[name]), path], { encoding: 'utf8' });
      if (result.error || result.status > 1) throw new Error('Could not print manifest diff');
      process.stdout.write(result.stdout);
    }
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
  if (dryRun) return;
  const bytes = Buffer.from(json(candidate['manifest.json']));
  const signature = signManifest(bytes, process.env.NEZZEL_MANIFEST_SIGNING_KEY, process.env.NEZZEL_MANIFEST_PUBKEY);
  const output = join(root, 'docs/manifest');
  await mkdir(output, { recursive: true });
  for (const [name, value] of Object.entries(candidate)) await writeFile(join(output, name), json(value));
  // No trailing newline: the Rust verifier decodes strict base64.
  await writeFile(join(output, 'manifest.sig'), signature);
  console.log('Signed manifest staged in docs/manifest (pinned inputs unchanged).');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
