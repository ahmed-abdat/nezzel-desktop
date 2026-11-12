# Signed tool manifest

The public repository `ahmed-abdat/nezzel-desktop` publishes `manifest.json` and
`manifest.sig` on the fixed `tool-manifest` release. Release builds use
`https://github.com/ahmed-abdat/nezzel-desktop/releases/download/tool-manifest/manifest.json`.
The signature is base64 Ed25519 over the exact JSON bytes, without a trailing
newline in the signature file. The build-time public key is 32 raw bytes in hex.

Owner setup (once; never run keygen in CI):

1. Run `node scripts/manifest-keygen.mjs /secure/path/manifest-private.key`.
   It creates a new file with mode 0600 and prints only the public key.
2. Keep an offline backup of that private file. Set its contents as the public
   repository's Actions secret `NEZZEL_MANIFEST_SIGNING_KEY` (base64 PKCS#8 DER).
3. Set repository variable `NEZZEL_MANIFEST_PUBKEY` to the printed public key.
   The workflow checks the signing key matches it before writing output.
4. Enable the Signed tool manifest workflow on the public repository when ready.

`node scripts/manifest-bump.mjs --dry-run` resolves official stable releases,
downloads and hashes every candidate asset, and prints unified diffs. It never
reads the signing key, signs, or writes published/pinned files. Empty, interrupted,
missing or failed downloads reject that tool's candidate. The previous pin is
kept for that tool and the others continue; the command fails if every source
failed. No partially refreshed tool is published. Successful automatic refreshes
clear a previous rollback flag; failed sources retain the entire previous pin. The normal invocation
requires the signing secret in the environment and stages output in
`docs/manifest/`; the workflow uploads it, without committing source changes.

The runtime manifest preserves existing extraction layouts and providers:
yt-dlp, yt-dlp/FFmpeg-Builds for Linux/Windows, Martin Riedl's release builds for
macOS, and denoland/deno. AtomicParsley remains pinned. gallery-dl (Codeberg) and
aria2 retain their separate build-manifest formats. These manifests are build-time
only and unsigned; refreshed copies are release assets for reviewing future
installer pin updates. Native builds and their Python/toolchain pins remain unchanged.

**At runtime, only yt-dlp follows the signed manifest today.** ffmpeg, ffprobe,
deno and the other bundled tools are updated with app releases. Publishing their
refreshed pins does not update installed copies of those tools.

When `NEZZEL_MANIFEST_PUBKEY` is configured at build time, yt-dlp uses only the
signed manifest; a missing URL, invalid signature/key, fetch failure or replay
falls back to the embedded snapshot. There is no upstream-checksum fallback in
this mode. Without a configured public key (including dev/local builds), the
original GitHub latest-release + `SHA2-256SUMS` update path remains active.

Remote bytes are parsed only after signature verification. Signed `generatedAt`
is required to be RFC3339. The last accepted timestamp is stored in tool metadata;
an older manifest is rejected, including after a restart. The timestamp is also
rechecked when committing an update, so an older in-flight download cannot replace
a newer accepted manifest. Equal timestamps are accepted; there is no expiry.
Archive hashes are checked before installation.

A candidate cannot be older than either the embedded or currently installed
yt-dlp version, unless that tool explicitly has `"rollback": true` in a verified
signed manifest. The default is false. Rollback authority is tied to the check and
the installed version observed before downloading; a concurrent change cancels it.

Publication is serialized. During replacement of the two release assets a reader
can see mismatched bytes and signature; it safely falls back until the next check.
The high-water timestamp rejects older signed manifests; it does not prevent a
server withholding newer manifests. There is no expiry or sequence counter.
Key rotation requires a new app build with the new public key.

For targeted Rust checks, prepare the isolation bundle via `pnpm run build:isolation`
(or copy the existing `app/dist-isolation` into a fresh worktree). No configuration
override is needed once `dist-isolation` exists:

```sh
PATH=$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH \
cargo test --manifest-path src-tauri/Cargo.toml -p nezzel --lib binaries:: -j 2
python3 -B -m unittest discover -s scripts -p test_fetch_tools.py
```
