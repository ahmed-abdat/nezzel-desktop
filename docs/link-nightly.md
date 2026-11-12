# Anonymous nightly link matrix

Implements [#31 part 1](https://github.com/ahmed-abdat/nezzel-desktop-dev/issues/31).
Part 2 belongs to [#65, batch problem reports](https://github.com/ahmed-abdat/nezzel-desktop-dev/issues/65);
this job does not implement reporting UI or send problem reports.
These are operator instructions; no server is provisioned by this change.

## Ubuntu 24.04 LTS, x86-64

Use a dedicated, unprivileged `nezzel-nightly` account. Put a checkout of the
reviewed source at `/opt/nezzel` (owned by that account). Keep the public input
list at `/var/lib/nezzel-nightly/links.tsv` and history at
`/var/lib/nezzel-nightly/history`. Do not mount browser profiles, cookies,
credential stores, `.env`, or authenticated/signed link lists into the job.
The job does not need a GitHub token. Fetch source outside the runtime container.

As the host administrator, install dependencies and create the work directory:

```sh
sudo useradd --system --create-home --home-dir /var/lib/nezzel-nightly --shell /bin/sh nezzel-nightly
sudo apt-get update
sudo apt-get install -y bash python3 ca-certificates curl xz-utils unzip \
  xvfb xauth dbus-x11 shellcheck \
  build-essential pkg-config libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev librsvg2-dev
sudo install -d -o nezzel-nightly -g nezzel-nightly /var/lib/nezzel-nightly
```

The harness is a real, windowless **Tauri/Wry** application. Linux still needs
GTK/WebKit runtime libraries, an X display, and a D-Bus session. `xvfb-run` and
`dbus-run-session` below supply those in a plain Ubuntu container; no desktop
session or privileged container is required. A prebuilt matrix binary avoids
Rust/Node/build tools at runtime, but still needs those shared libraries.

### Prepare the matrix binary

Choose **one**:

- Copy a trusted `link_matrix` executable built from this source with
  `--features link-matrix --example link_matrix` for `x86_64-unknown-linux-gnu`
  on Ubuntu 24.04 to `/opt/nezzel/bin/link_matrix`, then `chmod +x` it. The normal
  Nezzel GUI executable is not the matrix binary. Keep its matching source and
  sidecar manifests. Embedded isolation assets are included in the executable.
- Build on that Ubuntu host as `nezzel-nightly`, using the repository's Node
  version and pnpm pin, then the Rust toolchain:

```sh
cd /opt/nezzel
# Install the exact Node version from .nvmrc, verifying the official archive.
node_version=$(cat .nvmrc)
mkdir -p /var/lib/nezzel-nightly/node-download
cd /var/lib/nezzel-nightly/node-download
curl -fLO "https://nodejs.org/dist/v${node_version}/node-v${node_version}-linux-x64.tar.xz"
curl -fLO "https://nodejs.org/dist/v${node_version}/SHASUMS256.txt"
rg_line="node-v${node_version}-linux-x64.tar.xz"
awk -v name="$rg_line" '$2 == name {print}' SHASUMS256.txt > node.sha256
sha256sum -c node.sha256
tar -xJf "node-v${node_version}-linux-x64.tar.xz"
export PATH="$PWD/node-v${node_version}-linux-x64/bin:$PATH"
cd /opt/nezzel
# Install the pinned standalone pnpm CLI beside Node.
node_bin="/var/lib/nezzel-nightly/node-download/node-v${node_version}-linux-x64/bin"
curl -fL https://github.com/pnpm/pnpm/releases/download/v12.16.1/pnpm-linux-x64.tar.gz -o /var/lib/nezzel-nightly/pnpm.tar.gz
tar -xzf /var/lib/nezzel-nightly/pnpm.tar.gz -C "$node_bin"  # the release asset is a tarball, not a bare binary
chmod +x "$node_bin/pnpm"
pnpm install --frozen-lockfile
pnpm run build:isolation
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /var/lib/nezzel-nightly/rustup-init.sh
sh /var/lib/nezzel-nightly/rustup-init.sh -y --profile minimal --default-toolchain 1.99.1
. "$HOME/.cargo/env"
```

Prepare the sidecars below before compiling (Tauri's build also checks resources),
then finish the source-build option:

```sh
cd /opt/nezzel
# Release build: the debug build overflows its stack on real rows.
cargo build --release --locked --manifest-path src-tauri/Cargo.toml \
  --features link-matrix --example link_matrix -j 2
mkdir -p /opt/nezzel/bin /opt/nezzel/lib/Nezzel
install -m 755 src-tauri/target/release/examples/link_matrix /opt/nezzel/bin/link_matrix
# A release binary looks for its tools like a packaged app does (else every row
# fails with browserCookiesUnreadable): give it that layout.
ln -sfn /opt/nezzel/src-tauri/binaries/tools /opt/nezzel/lib/Nezzel/tools
```

### Prepare pinned linux-x64 sidecars (both options)

Run as `nezzel-nightly`; Python uses only its standard library for these downloads.
This must succeed; do not use the development `--allow-unavailable` gallery stub.

```sh
cd /opt/nezzel
python3 scripts/fetch-gallery-dl.py --target x86_64-unknown-linux-gnu
python3 scripts/fetch-tools.py --target x86_64-unknown-linux-gnu
mkdir -p /var/lib/nezzel-nightly/bin /var/lib/nezzel-nightly/resources
ln -sfn /opt/nezzel/src-tauri/binaries/tools/yt-dlp/yt-dlp \
  /var/lib/nezzel-nightly/bin/yt-dlp
for tool in ffmpeg ffprobe deno; do
  ln -sfn "/opt/nezzel/src-tauri/binaries/${tool}-x86_64-unknown-linux-gnu" \
    "/var/lib/nezzel-nightly/bin/$tool"
done
ln -sfn /opt/nezzel/src-tauri/binaries/gallery-dl-x86_64-unknown-linux-gnu \
  /var/lib/nezzel-nightly/resources/gallery-dl
ln -sfn /opt/nezzel/src-tauri/binaries/gallery-dl.sha256 \
  /var/lib/nezzel-nightly/resources/gallery-dl.sha256
ln -sfn /opt/nezzel/src-tauri/binaries/tools /var/lib/nezzel-nightly/resources/tools
```

Keep the entire yt-dlp onedir payload; copying its executable alone breaks Python
loading. Required binaries are **yt-dlp, ffmpeg, ffprobe, deno, gallery-dl**, all
linux-x64, not macOS copies or system substitutes. The fetch scripts check pinned
archive hashes in `src-tauri/src/binaries/manifest.json` and
`scripts/gallery-dl-manifest.json`. Gallery's executable additionally must match
`gallery-dl.sha256` at runtime; missing/mismatched receipts fail harness setup.
Refresh sidecars deliberately with the matching checkout, outside a running job.

For source builds, Tauri also expects its optional aria2 external-bin resource:
`python3 scripts/fetch-aria2.py --target x86_64-unknown-linux-gnu` (builds aria2 from
source; there is no `--allow-unavailable` flag; the matrix itself does not use aria2).
Do this before `cargo build`.

## Input and one manual run

Input uses tab-separated `site`, `label`, `expected_type`, `url` columns; `note`
is optional. Header order is flexible; legacy headerless rows use that four-column
order. Comments start with `#`. Duplicate nonempty URLs use the first row. Blank
URLs remain explicit FAIL coverage gaps. Input lists stay outside the repository.

Normal media expectations keep the existing stream/count checks. Additional rules:

- `gone`: PASS only for a failed pipeline with terminal app code `notFound404`
  (removed/not found) or `geoBlocked`, and no saved artifacts.
- `none`: PASS only for a failed pipeline with terminal `noMediaFound` and no
  saved artifacts. Cancellation, empty metadata, timeouts, 403/429, and prose
  without the matching app code never satisfy either error expectation. Earlier
  fallback errors do not count. Ambiguous private/unavailable codes stay non-PASS.
- `file`: a new, nonempty regular output whose magic matches its filename suffix;
  no ffprobe media-stream requirement. Supported suffixes: `pdf`, `zip`, `exe`,
  `dll`, `msi`, `7z`, `rar`, `gz`, `tgz`, `bz2`, `xz`, `tar` (ustar). Unknown
  signatures/extensions fail closed. This checks type, not full archive integrity.

Run under the dedicated account, after supplying the public list:

```sh
cd /opt/nezzel
export LINK_MATRIX_BINARY=/opt/nezzel/bin/link_matrix
export LINK_MATRIX_BIN_DIR=/var/lib/nezzel-nightly/bin
export LINK_MATRIX_RESOURCES_DIR=/var/lib/nezzel-nightly/resources
xvfb-run -a dbus-run-session -- scripts/link-nightly.sh \
  --links /var/lib/nezzel-nightly/links.tsv \
  --out /var/lib/nezzel-nightly/history
```

`--matrix-bin`, `--bin-dir`, and `--resources-dir` override those environment
variables. The matrix runs with `CI=1`, fresh temporary HOME/XDG config/data/cache,
working directory inside that HOME, and no inherited secrets. Only PATH, DISPLAY,
XAUTHORITY, DBUS_SESSION_BUS_ADDRESS, XDG_RUNTIME_DIR, and LD_LIBRARY_PATH are
forwarded, plus the selected sidecar directories. Use system PATH/library paths
and fresh Xvfb/D-Bus sessions. Production cookie discovery is disabled; no login
or cookie flags are accepted by the wrapper. The total matrix deadline is six
hours (`--timeout SECONDS`); expiry kills its process group, including engines.

History is `history/anonymous/YYYY-MM-DDTHHMMSS.ffffffZ/`, with `results.tsv`,
`diff.tsv`, `regressions.tsv`, `links.tsv`, matrix evidence/downloads and
`matrix.log`. Reports include public URLs and local output paths: keep history
private to the operator. The first run establishes the baseline. The diff compares
**verdicts**, keyed by exact URL, regardless of row order or changed timing/paths.
Changed expectations reset that row's baseline; removed inputs appear as REMOVED
but are not regressions. A zero-PASS previous run warns on stderr.

Exit codes: **0** completed without regressions, **1** PASS→non-PASS regressions
(printed to stdout), **2** setup/timeout/invalid/partial results. Only fully
validated anonymous results receive an atomic `complete` marker and can become a
baseline. Failed/incomplete processes preserve artifacts but leave the previous
baseline intact. A completed regression run becomes the next baseline, so the
same continuing failure does not notify every night. An exclusive, nonblocking
`fcntl.flock` on `history/.lock` prevents overlap. The kernel releases the lock on
exit, crash, or SIGKILL; the lock file remains and must not be deleted. SIGTERM
and SIGHUP stop and reap the matrix process group before exiting. Do not compare runs from different regions/egress
policies using the same history.

## Cron at 03:00 and notifications

As `nezzel-nightly`, create `/var/lib/nezzel-nightly/run.sh`:

```sh
#!/bin/sh
set -eu
export PATH=/usr/local/bin:/usr/bin:/bin
export LINK_MATRIX_BINARY=/opt/nezzel/bin/link_matrix
export LINK_MATRIX_BIN_DIR=/var/lib/nezzel-nightly/bin
export LINK_MATRIX_RESOURCES_DIR=/var/lib/nezzel-nightly/resources
# Operator-supplied shell command, called only for status 1 or 2.
export NOTIFY_CMD='logger -t nezzel-nightly "status=$LINK_NIGHTLY_STATUS results=$LINK_NIGHTLY_RESULTS diff=$LINK_NIGHTLY_DIFF"'
exec xvfb-run -a dbus-run-session -- /opt/nezzel/scripts/link-nightly.sh \
  --links /var/lib/nezzel-nightly/links.tsv --out /var/lib/nezzel-nightly/history
```

```sh
chmod 700 /var/lib/nezzel-nightly/run.sh
crontab -e
```

Add (03:00 in the Ubuntu host's configured timezone; check with `timedatectl`):

```cron
0 3 * * * /var/lib/nezzel-nightly/run.sh >> /var/lib/nezzel-nightly/cron.log 2>&1
```

`NOTIFY_CMD` runs via `/bin/sh -c` in the operator environment, after the matrix
has exited. It receives `LINK_NIGHTLY_STATUS`, `LINK_NIGHTLY_DIR`,
`LINK_NIGHTLY_RESULTS`, `LINK_NIGHTLY_DIFF`, `LINK_NIGHTLY_REGRESSIONS` (absolute
paths; may be empty or not yet created on setup failure). Quote these variables.
Replace `logger` with your notification command. No notification command or its
credentials reach the matrix. The hook starts in a new process session; its
60-second timeout kills the entire process group and reaps the hook process.
A failed/timed-out notification prints an error and preserves the original
nonzero status and results. An unset command disables the
hook; cron still captures the failure output.

## Plain Linux container

Use the same Ubuntu dependencies and matching prebuilt linux-x64 matrix/sidecars
inside a dedicated Ubuntu 24.04 image. Copy source scripts and manifests, the
matrix executable, and the sidecar payloads into that image using the paths above.
Run as an unprivileged UID with a writable `/var/lib/nezzel-nightly`; mount only
that history and the public input list. Entry command:

```sh
xvfb-run -a dbus-run-session -- /opt/nezzel/scripts/link-nightly.sh \
  --matrix-bin /opt/nezzel/bin/link_matrix \
  --bin-dir /var/lib/nezzel-nightly/bin \
  --resources-dir /var/lib/nezzel-nightly/resources \
  --links /var/lib/nezzel-nightly/links.tsv --out /var/lib/nezzel-nightly/history
```

Schedule this container from host cron at 03:00 instead of the direct command if
preferred. Keep history on a persistent volume between containers. No systemd,
Docker socket, desktop session, or privileged mode is needed inside the container.

## Local offline done-check

No extractors, Rust build, display, network, or cookies are used by `--dry-run`.
`fixture_verdict` defaults to PASS and is accepted **only** in dry runs. These
results live under `history/dry-run/`, never the real anonymous baseline.

```sh
sample_dir=$(mktemp -d)
printf 'site\tlabel\texpected_type\turl\tfixture_verdict\nexample\tclip\tvideo\thttps://example.com/1\tPASS\nexample\ttext\tnone\thttps://example.com/2\tPASS\nexample\tdoc\tfile\thttps://example.com/3\tPASS\n' > "$sample_dir/links.tsv"
scripts/link-nightly.sh --dry-run --links "$sample_dir/links.tsv" --out "$sample_dir/history"
sed -i.bak 's@https://example.com/2\tPASS@https://example.com/2\tFAIL@' "$sample_dir/links.tsv"
# Expected exit 1; prints the one regression and writes a second results.tsv + diff.
scripts/link-nightly.sh --dry-run --links "$sample_dir/links.tsv" --out "$sample_dir/history"
shellcheck scripts/link-nightly.sh
python3 -B -m unittest discover -s scripts -p test_link_nightly.py
# Existing verifier test module, compiled directly to avoid building the GUI/dependencies:
rustc --edition=2021 --test src-tauri/examples/link_matrix/support.rs -o "$sample_dir/verifier-tests"
"$sample_dir/verifier-tests"
```

With the normal app build prerequisites available, the same verifier tables also
run via `cargo test --manifest-path src-tauri/Cargo.toml --features link-matrix
--lib link_matrix::support::tests -j 2`. Wrapper tests are included in the existing
`pnpm run test:tools` discovery gate (Rust CI).
