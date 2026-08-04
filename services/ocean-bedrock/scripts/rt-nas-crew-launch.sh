#!/usr/bin/env bash
set -euo pipefail

ROOT="/root/rt-nas/rt-nas"
SESSION="rt-nas-crew"
MODEL="${RT_NAS_AGENT_MODEL:-deepseek/deepseek-v4-flash:low}"
PI_BIN="${PI_BIN:-pi}"
SESSION_DIR="$ROOT/.pi/sessions"
LOG_DIR="$ROOT/.pi/crew-logs"

mkdir -p "$SESSION_DIR" "$LOG_DIR"
cd "$ROOT"

agent_prompt() {
  local name="$1"
  local callsign="$2"
  local file="$3"
  cat <<PROMPT
You are ${name} — ${callsign}, an Ocean Bedrock rt-nas agent.

Scope is strictly /root/rt-nas/rt-nas unless the human explicitly authorizes otherwise.
Read your starter instructions from ${file} and the crew boot notes from docs/RT-NAS-CREW-BOOT.md.

First, call pi_messenger({ action: "join" }), then call pi_messenger({ action: "rename", name: "${name}" }), then call pi_messenger({ action: "status" }).
If join/rename/status fails due to stale messenger state, report the issue briefly and remain available.
Do not call pi_messenger({ action: "work" }) unless explicitly assigned.
After status, wait interactively for assignment.
PROMPT
}

start_agent() {
  local window="$1"
  local name="$2"
  local callsign="$3"
  local file="$4"
  local prompt
  prompt="$(agent_prompt "$name" "$callsign" "$file")"

  tmux new-window -t "$SESSION" -n "$window" "cd '$ROOT' && export PI_CODING_AGENT_SESSION_DIR='$SESSION_DIR' && export RT_NAS_AGENT_NAME='$name' && export RT_NAS_AGENT_CALLSIGN='$callsign' && $PI_BIN --model '$MODEL' --append-system-prompt '$file' \"$prompt\" 2>&1 | tee -a '$LOG_DIR/${callsign}.log'"
}

if tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "tmux session '$SESSION' already exists."
  echo "Attach with: tmux attach -t $SESSION"
  exit 0
fi

# Holding window for operator notes.
tmux new-session -d -s "$SESSION" -n "README" "cd '$ROOT' && export PI_CODING_AGENT_SESSION_DIR='$SESSION_DIR' && printf '%s\n' 'rt-nas crew launched.' 'Attach: tmux attach -t rt-nas-crew' 'Agents: NAV ANK KEL SNR BCN' 'All cwd-scoped to /root/rt-nas/rt-nas.' && exec bash"

start_agent "NAV" "Navigator" "NAV" "docs/agents/NAV.md"
start_agent "ANK" "Anchor" "ANK" "docs/agents/ANK.md"
start_agent "KEL" "Keel" "KEL" "docs/agents/KEL.md"
start_agent "SNR" "Sonar" "SNR" "docs/agents/SNR.md"
start_agent "BCN" "Beacon" "BCN" "docs/agents/BCN.md"

tmux select-window -t "$SESSION:NAV"

echo "Started rt-nas core crew in tmux session '$SESSION'."
echo "Attach with: tmux attach -t $SESSION"
echo "Logs: $LOG_DIR"
