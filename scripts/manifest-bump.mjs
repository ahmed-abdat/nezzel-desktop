#!/usr/bin/env node
import { createHash, createPrivateKey, sign, verify, createPublicKey } from 'node:crypto';
import { readFile, writeFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const json = value => `${JSON.stringify(value, null, 2)}\n`;

async function request(url, fetcher = fetch) {
  if (new URL(url).protocol !== 'https:') throw new Error('Tool sources must use HTTPS');
  const headers = { 'User-Agent': 'Nezzel-manifest/1.0 (+https://github.com/ahmed-abdat/nezzel-desktop)' };
  // Never send the repository token to a tool host/CDN.
  if (new URL(url).origin === 'https://api.github.com' && process.env.GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  }
  const response = await fetcher(url, { headers, signal: AbortSignal.timeout(300_000) });
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

  let ffRelease;
  async function getFfRelease() {
    ffRelease ??= (async () => {
      // /latest is a moving alias; preserve the immutable autobuild layout.
      const releases = await (await request('https://api.github.com/repos/yt-dlp/FFmpeg-Builds/releases?per_page=10', fetcher)).json();
      const release = releases.find(item => /^autobuild-/.test(item.tag_name)
        && !item.draft && !item.prerelease && item.assets?.length);
      if (!release) throw new Error('No immutable ffmpeg autobuild release');
      return release;
    })();
    return ffRelease;
  }
  for (const name of ['ffmpeg', 'ffprobe']) {
    manifest.tools[name] = await refresh(name, manifest.tools[name], async (tool) => {
      const ff = await getFfRelease();
      for (const [platform, file] of Object.entries(tool.files)) {
        if (platform.startsWith('darwin-')) {
          const arch = platform.endsWith('aarch64') ? 'arm64' : 'amd64';
          const response = await request(`https://ffmpeg.martin-riedl.de/redirect/latest/macos/${arch}/release/${name}.zip`, fetcher);
          // Resolve the existing provider's stable redirect to a versioned, immutable URL.
          const match = new URL(response.url).pathname.match(/^\/download\/macos\/(amd64|arm64)\/\d+_([^/]+)\/(ffmpeg|ffprobe)\.zip$/);
          await response.body?.cancel();
          if (!match || new URL(response.url).hostname !== 'ffmpeg.martin-riedl.de') throw new Error('Unpinned macOS ffmpeg release');
          file.url = response.url;
          file.version = match[2];
        } else {
          const suffix = { 'linux-x86_64': 'linux64-gpl.tar.xz', 'linux-aarch64': 'linuxarm64-gpl.tar.xz',
            'windows-x86_64': 'win64-gpl.zip', 'windows-aarch64': 'winarm64-gpl.zip' }[platform];
          const matches = ff.assets.filter(item => item.name.startsWith('ffmpeg-N-') && item.name.endsWith(`-${suffix}`));
          if (!suffix || matches.length !== 1) throw new Error(`Missing pinned ffmpeg asset: ${platform}`);
          const filename = matches[0].name;
          file.url = asset(ff, filename);
          file.version = filename.slice(7, -suffix.length - 1);
          const folder = filename.replace(/\.(tar\.xz|zip)$/, '');
          file.entry = `${folder}/bin/${name}${platform.startsWith('windows-') ? '.exe' : ''}`;
        }
        file.sha256 = await hashed(file.url, hashes, fetcher);
      }
      tool.version = [...new Set(Object.values(tool.files).map(file => file.version))].join(' / ');
    });
  }
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

async function main() {
  const args = process.argv.slice(2);
  if (args.some(arg => arg !== '--dry-run')) throw new Error('Usage: node scripts/manifest-bump.mjs [--dry-run]');
  const dryRun = args.includes('--dry-run');
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
