#!/usr/bin/env bash
# Run a CI step; on failure, surface the last lines of output as a GitHub
# annotation so the cause is visible in the run summary (and via the API).
set -o pipefail
log=$(mktemp)
"$@" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}
if [[ $status -ne 0 ]]; then
  tail_text=$(grep -vE "ALSA|[Jj]ack|Cannot connect|libEGL|dbind" "$log" | grep -E "error|panicked|warning: unused|FAIL|x |-->" | tail -25)
  [[ -z "$tail_text" ]] && tail_text=$(tail -25 "$log")
  # Annotations need %, CR and LF escaped.
  esc=${tail_text//'%'/'%25'}; esc=${esc//$'\r'/'%0D'}; esc=${esc//$'\n'/'%0A'}
  echo "::error title=$1 failed::$esc"
fi
exit $status
