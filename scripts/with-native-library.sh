#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -lt 2 ]]; then
  echo "usage: $0 <native-directory> <command> [arguments...]" >&2
  exit 2
fi

native_dir="$1"
shift
case "$(uname -s)" in
  Linux)
    export LD_LIBRARY_PATH="${native_dir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
    ;;
  Darwin)
    export DYLD_LIBRARY_PATH="${native_dir}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}"
    ;;
  *)
    echo "unsupported native-library host: $(uname -s)" >&2
    exit 1
    ;;
esac
exec "$@"
