#!/usr/bin/env bash
# Stand-in C compiler for `cargo check` against a foreign target (no SDK
# installed). Build scripts only need object files to exist; `check` never
# links them. Do not use for real builds.
out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -Fo*) out="${1#-Fo}"; shift ;;
    # cc-rs probes `-?` to detect cl.exe; answer like cl only for MSVC targets.
    -\?) [[ "${FAKE_CC_MSVC:-0}" == 1 ]] && exit 0 || exit 1 ;;
    --version) exit 1 ;;
    *) shift ;;
  esac
done
[[ -n "$out" ]] && : > "$out"
exit 0
