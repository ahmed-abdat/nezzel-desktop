#!/usr/bin/env node
// Private: base64 PKCS#8 DER. Public: raw 32-byte Ed25519 key, hex (Rust verifier).
import { generateKeyPairSync } from 'node:crypto';
import { writeFile, chmod } from 'node:fs/promises';

const path = process.argv[2];
if (!path || process.argv.length !== 3) {
  console.error('Usage: node scripts/manifest-keygen.mjs PRIVATE_KEY_FILE');
  process.exit(1);
}
try {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  await writeFile(path, privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'),
    { mode: 0o600, flag: 'wx' });
  await chmod(path, 0o600);
  console.log(Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url').toString('hex'));
} catch {
  console.error('Key generation failed (output must be a new, writable file).');
  process.exitCode = 1;
}
