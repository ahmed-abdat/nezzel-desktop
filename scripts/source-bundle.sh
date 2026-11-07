#!/bin/sh
# Like Android's source-bundle.sh: <version> <outdir>; --dry-run is entirely offline.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 -B "$script_dir/source-bundle.py" "$@"
