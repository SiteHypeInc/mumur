#!/usr/bin/env bash
# Type-check the whole workspace for macOS and Windows from a Linux box.
set -euo pipefail
cd "$(dirname "$0")/.."
FAKE="$PWD/scripts/fake-cc.sh"
for target in aarch64-apple-darwin x86_64-pc-windows-msvc; do
  echo "== $target"
  t=${target//-/_}
  msvc=0; [[ "$target" == *msvc ]] && msvc=1
  env FAKE_CC_MSVC=$msvc "CC_$t=$FAKE" "CXX_$t=$FAKE" "AR_$t=$PWD/scripts/fake-ar.sh" cargo check --workspace --target "$target" "$@"
done
