import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const OUTPUT_DIR = 'licenses';
const NPM_FILE = join(OUTPUT_DIR, 'licenses-npm.json');
const RUST_FILE = join(OUTPUT_DIR, 'licenses-rust.txt');
const OUTPUT_FILE = join(OUTPUT_DIR, '3rdpartylicenses.txt');

type LicenseInfo = {
  name?: string;
  version?: string;
  licenses?: string | string[];
  licenseFile?: string;
};

function readFileOrEmpty(path: string): string {
  try {
    return readFileSync(path, 'utf8').trim();
  } catch {
    console.warn(`[licenses] missing file: ${path}`);
    return '';
  }
}

async function main() {
  if (!existsSync(OUTPUT_DIR)) {
    mkdirSync(OUTPUT_DIR);
  }

  const rust = readFileOrEmpty(RUST_FILE);
  const npm = generateNpmLicenses();
  const externalTools = await generateExternalToolsSection();

  const content = [
    'Nezzel – Third-Party Licenses',
    '===========================================',
    '',
    externalTools,
    '',
    rust ? 'RUST DEPENDENCIES\n-----------------\n\n' + rust + '\n' : '',
    npm ? '\nNPM DEPENDENCIES\n----------------\n\n' + npm + '\n' : '',
    '',
  ].join('\n');

  writeFileSync(OUTPUT_FILE, content, 'utf8');
  console.log(`[licenses] wrote ${OUTPUT_FILE}`);
}

function generateNpmLicenses() {
  const raw = readFileSync(NPM_FILE, 'utf8');
  const data: Record<string, LicenseInfo> = JSON.parse(raw);

  const packageLines: string[] = [];
  const licenseTextMap = new Map<string, string>();

  for (const [key, info] of Object.entries(data)) {
    const name = info.name || key.split('@')[0] || key;
    const version
      = info.version
        || key
          .split('@')
          .slice(1)
          .join('@')
          || 'unknown';

    const licenses = Array.isArray(info.licenses)
      ? info.licenses
      : info.licenses
        ? [info.licenses]
        : ['UNKNOWN'];

    for (const lic of licenses) {
      packageLines.push(`${name} ${version} - ${lic}`);

      if (info.licenseFile && !licenseTextMap.has(lic)) {
        try {
          const text = readFileSync(info.licenseFile, 'utf8');
          licenseTextMap.set(lic, text.trim());
        } catch (e) {
          console.warn(`[licenses] could not read license file ${info.licenseFile}: ${e}.`);
        }
      }
    }
  }

  packageLines.sort((a, b) => a.localeCompare(b));

  const licenseTextBlocks: string[] = [];
  licenseTextMap.forEach((text, id) => {
    licenseTextBlocks.push(`${id} - ${id}`);
    licenseTextBlocks.push(text);
    licenseTextBlocks.push('');
    licenseTextBlocks.push('----------------------------------------');
    licenseTextBlocks.push('');
  });

  return [
    'PACKAGES',
    '========',
    '',
    packageLines.join('\n'),
    '',
    'LICENSE TEXTS',
    '============',
    '',
    licenseTextBlocks.join('\n').trim(),
    '',
  ].join('\n');
}

async function generateExternalToolsSection() {
  const lines: string[] = [];
  lines.push('EXTERNAL TOOLS');
  lines.push('-----------------');
  lines.push('');
  lines.push('This application bundles external tools; verified updates and repairs use app-data.');
  lines.push('These tools are separate projects with their own licenses.');
  lines.push('Installer versions below come from the pinned manifest; yt-dlp updates may be newer.');
  lines.push('');

  const aria2 = JSON.parse(readFileSync('scripts/aria2-manifest.json', 'utf8')) as {
    version: string; license: string; source: { url: string };
  };
  lines.push(`aria2 (${aria2.version}) — bundled sidecar`);
  lines.push(`  License: ${aria2.license} (GPL-2.0+)`);
  lines.push(`  Corresponding source: ${aria2.source.url}`);
  lines.push('  Build recipe: scripts/fetch-aria2.py (native AppleTLS/static OpenSSL builds)');
  lines.push(readFileOrEmpty('scripts/aria2-COPYING'));
  lines.push('');

  const gallery = JSON.parse(readFileSync('scripts/gallery-dl-manifest.json', 'utf8')) as {
    version: string; license: string; source: { url: string };
  };
  lines.push(`gallery-dl (${gallery.version}) — separate bundled executable`);
  lines.push(`  License: ${gallery.license}`);
  lines.push(`  Corresponding source: ${gallery.source.url}`);
  lines.push('  Build recipe and pinned Python environment: scripts/fetch-gallery-dl.py, scripts/gallery-dl-build-requirements.txt');
  lines.push('  This executable is distributed as a separate program, not linked into Nezzel.');
  lines.push(readFileOrEmpty('scripts/gallery-dl-LICENSE'));
  lines.push('');
  lines.push(readFileSync('scripts/gallery-dl-bundled-NOTICES.txt', 'utf8'));
  lines.push('');

  const manifest = JSON.parse(readFileSync('src-tauri/src/binaries/manifest.json', 'utf8')) as {
    tools: Record<string, { version: string; files: Record<string, { version?: string }> }>;
  };
  for (const name of ['yt-dlp', 'ffmpeg', 'ffprobe', 'deno']) {
    const tool = manifest.tools[name];
    lines.push(`${name} (${tool.version}) — bundled installer tool`);
    for (const [platform, file] of Object.entries(tool.files)) {
      if (file.version) lines.push(`  ${platform}: ${file.version}`);
    }
  }
  for (const path of ['licenses/tools-NOTICES.txt', 'scripts/ffmpeg-COPYING.GPLv3',
    'scripts/deno-LICENSE', 'scripts/yt-dlp-LICENSE', 'scripts/yt-dlp-bundled-NOTICES.txt']) {
    lines.push('');
    lines.push(readFileSync(path, 'utf8'));
  }
  lines.push('AtomicParsley is an optional repair download, GPL-2.0-or-later.');
  lines.push('Source: https://github.com/wez/atomicparsley');

  return lines.join('\n');
}

void main();
