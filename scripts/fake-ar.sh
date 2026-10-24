#!/usr/bin/env bash
# Stand-in archiver for scripts/fake-cc.sh: creates an empty archive at the
# output path, whether called GNU-style (`cq out.a ...`) or lib.exe-style (`-out:out.lib`).
for a in "$@"; do
  case "$a" in
    -out:*|/OUT:*) : > "${a#*:}"; exit 0 ;;
    *.a|*.lib) : > "$a"; exit 0 ;;
  esac
done
exit 0
