#!/usr/bin/env bash
# 3 agents independently answer same question, judge picks best.
set -euo pipefail

LAUNCHED_NAMES=()
track_launch() {
  local names=$(echo "$1" | grep '^Names: ' | sed 's/^Names: //')
  for n in $names; do LAUNCHED_NAMES+=("$n"); done
}
cleanup() {
  for name in "${LAUNCHED_NAMES[@]}"; do
    comms kill "$name" --go 2>/dev/null || true
  done
}

name_flag=""
task=""
while [[ $# -gt 0 ]]; do
  case "$1" in --name) name_flag="$2"; shift 2 ;; -*) shift ;; *) task="$1"; shift ;; esac
done
name_arg=""
[[ -n "$name_flag" ]] && name_arg="--name $name_flag"
task="${task:-what is 17 * 23}"
thread="ens-$(date +%s)"

trap cleanup ERR INT TERM

for i in 1 2 3; do
  launch_out=$(comms 1 claude --tag "c${i}" --go --headless \
    --comms-prompt "Answer: ${task}. Send ONLY your answer: comms send \"@judge-\" --thread ${thread} --intent inform -- \"C${i}: <answer>\". Then stop: comms stop" 2>&1)
  track_launch "$launch_out"
  name=$(echo "$launch_out" | grep '^Names: ' | sed 's/^Names: //' | tr -d ' ')
  echo "Contestant $i: $name"
done

launch_out=$(comms 1 claude --tag judge --go --headless \
  --comms-prompt "Wait for 3 answers in thread '${thread}'. Check: comms events --sql \"msg_thread='${thread}'\" --last 10. Pick best. Send: comms send \"@bigboss\" --thread ${thread} --intent inform -- \"VERDICT: <winner>\". Then stop: comms stop" 2>&1)
track_launch "$launch_out"
judge=$(echo "$launch_out" | grep '^Names: ' | sed 's/^Names: //' | tr -d ' ')
echo "Judge: $judge"
echo "Thread: $thread"
echo "Waiting..."

comms events --wait 120 --sql "type='message' AND msg_thread='${thread}' AND msg_text LIKE '%VERDICT%'" $name_arg >/dev/null 2>&1 && echo "PASS" || echo "FAIL"

trap - ERR
for name in "${LAUNCHED_NAMES[@]}"; do comms kill "$name" --go 2>/dev/null || true; done
comms events --sql "msg_thread='${thread}'" --last 10 2>&1
