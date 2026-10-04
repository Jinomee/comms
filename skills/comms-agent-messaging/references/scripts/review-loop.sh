#!/usr/bin/env bash
# Worker does task, reviewer reads transcript and gives feedback. Loop until approved.
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
task="${task:-count from 1 to 10 in Turkish}"
thread="review-$(date +%s)"

trap cleanup ERR INT TERM

launch_out=$(comms 1 claude --tag worker --go --headless \
  --comms-prompt "Task: ${task}. Do it, then send: comms send \"@reviewer-\" --thread ${thread} --intent inform -- \"ROUND 1 DONE: <result>\". If you get feedback, fix and resend as ROUND 2 DONE. After APPROVED, send: comms send \"@bigboss\" --thread ${thread} --intent inform -- \"FINAL\". Then stop: comms stop" 2>&1)
track_launch "$launch_out"
worker=$(echo "$launch_out" | grep '^Names: ' | sed 's/^Names: //' | tr -d ' ')
echo "Worker: $worker"

launch_out=$(comms 1 claude --tag reviewer --go --headless \
  --comms-prompt "You review @worker- output. On ROUND N DONE message, check the result. If correct: comms send \"@${worker}\" --thread ${thread} --intent inform -- \"APPROVED\". If wrong: comms send \"@${worker}\" --thread ${thread} --intent request -- \"FIX: <issue>\". Then stop: comms stop" 2>&1)
track_launch "$launch_out"
reviewer=$(echo "$launch_out" | grep '^Names: ' | sed 's/^Names: //' | tr -d ' ')
echo "Reviewer: $reviewer"
echo "Thread: $thread"
echo "Waiting..."

comms events --wait 120 --sql "type='message' AND msg_thread='${thread}' AND msg_text LIKE '%FINAL%'" $name_arg >/dev/null 2>&1 && echo "PASS" || echo "FAIL"

trap - ERR
for name in "${LAUNCHED_NAMES[@]}"; do comms kill "$name" --go 2>/dev/null || true; done
comms events --sql "msg_thread='${thread}'" --last 10 2>&1
