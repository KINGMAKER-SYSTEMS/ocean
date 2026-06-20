#!/usr/bin/env bash
set -euo pipefail

ROOT="/root/rt-nas/rt-nas"
SESSION="${RT_NAS_CREW_SESSION:-rt-nas-crew}"
MODEL="${RT_NAS_AGENT_MODEL:-deepseek/deepseek-v4-flash:low}"
PI_BIN="${PI_BIN:-pi}"
SESSION_DIR="$ROOT/.pi/sessions"
LOG_DIR="$ROOT/.pi/crew-logs"
PROMPT_DIR="$ROOT/.pi/crew-prompts"
REPLACE=0

usage() {
  cat <<USAGE
Usage: $0 [--replace]

Launch the Ocean Bedrock rt-nas core crew in a single tmux cockpit:
  left:   Anchor / Keel
  center: Navigator (orchestrator)
  right:  Sonar / Beacon

Options:
  --replace, -r   Kill and rebuild the existing $SESSION tmux session.
  --help, -h      Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --replace|-r)
      REPLACE=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

mkdir -p "$SESSION_DIR" "$LOG_DIR" "$PROMPT_DIR"
cd "$ROOT"

agent_prompt() {
  local name="$1"
  local callsign="$2"
  local file="$3"
  local mesh_name="${name}-${callsign}"
  cat <<PROMPT
You are ${name} — ${callsign}, an Ocean Bedrock rt-nas agent.

Scope is strictly /root/rt-nas/rt-nas unless the human explicitly authorizes otherwise.
Read your starter instructions from ${file} and the crew boot notes from docs/RT-NAS-CREW-BOOT.md.

First, call pi_messenger({ action: "join" }), then call pi_messenger({ action: "rename", name: "${mesh_name}" }), then call pi_messenger({ action: "status" }).
If join/rename/status fails due to stale messenger state, report the issue briefly and remain available.
Do not call pi_messenger({ action: "work" }) unless explicitly assigned.
After status, wait interactively for assignment.
PROMPT
}

pane_title() {
  local callsign="$1"
  case "$callsign" in
    NAV) printf '%s\n' "🧭 NAV · Navigator / orchestrator" ;;
    ANK) printf '%s\n' "⚓ ANK · Anchor / source" ;;
    KEL) printf '%s\n' "⛵ KEL · Keel / runtime" ;;
    SNR) printf '%s\n' "◉ SNR · Sonar / context" ;;
    BCN) printf '%s\n' "✦ BCN · Beacon / health" ;;
    *) printf '%s\n' "$callsign" ;;
  esac
}

agent_command() {
  local name="$1"
  local callsign="$2"
  local file="$3"
  local prompt_file="$PROMPT_DIR/${callsign}.prompt"
  agent_prompt "$name" "$callsign" "$file" > "$prompt_file"
  cat <<CMD
cd '$ROOT' && export PI_CODING_AGENT_SESSION_DIR='$SESSION_DIR' RT_NAS_AGENT_NAME='$name' RT_NAS_AGENT_CALLSIGN='$callsign' OCEAN_BEDROCK_MODE=1 && prompt=\$(cat '$prompt_file') && $PI_BIN --model '$MODEL' --append-system-prompt '$file' "\$prompt" 2>&1 | tee -a '$LOG_DIR/${callsign}.log'
CMD
}

clean_spawned_presence() {
  rm -f \
    "$ROOT/.pi/messenger/live/agents/Navigator-NAV.json" \
    "$ROOT/.pi/messenger/live/agents/Anchor-ANK.json" \
    "$ROOT/.pi/messenger/live/agents/Keel-KEL.json" \
    "$ROOT/.pi/messenger/live/agents/Sonar-SNR.json" \
    "$ROOT/.pi/messenger/live/agents/Beacon-BCN.json"
}

if tmux has-session -t "$SESSION" 2>/dev/null; then
  if [[ "$REPLACE" -eq 1 ]]; then
    tmux kill-session -t "$SESSION"
  else
    echo "tmux session '$SESSION' already exists."
    echo "Attach with: tmux attach -t $SESSION"
    echo "Or rebuild with: $0 --replace"
    exit 0
  fi
fi

clean_spawned_presence

# Centered cockpit layout, Tide-mesh style:
#   ┌─────────────┬──────────────────────────┬─────────────┐
#   │ Anchor      │                          │ Sonar       │
#   ├─────────────┤       Navigator          ├─────────────┤
#   │ Keel        │       orchestrator       │ Beacon      │
#   └─────────────┴──────────────────────────┴─────────────┘
nav_pane="$(tmux new-session -d -s "$SESSION" -n "Crew" -P -F '#{pane_id}' "$(agent_command "Navigator" "NAV" "docs/agents/NAV.md")")"
tmux select-pane -t "$nav_pane" -T "$(pane_title NAV)"

ank_pane="$(tmux split-window -d -h -b -p 30 -t "$nav_pane" -P -F '#{pane_id}' "$(agent_command "Anchor" "ANK" "docs/agents/ANK.md")")"
tmux select-pane -t "$ank_pane" -T "$(pane_title ANK)"

kel_pane="$(tmux split-window -d -v -p 50 -t "$ank_pane" -P -F '#{pane_id}' "$(agent_command "Keel" "KEL" "docs/agents/KEL.md")")"
tmux select-pane -t "$kel_pane" -T "$(pane_title KEL)"

snr_pane="$(tmux split-window -d -h -p 43 -t "$nav_pane" -P -F '#{pane_id}' "$(agent_command "Sonar" "SNR" "docs/agents/SNR.md")")"
tmux select-pane -t "$snr_pane" -T "$(pane_title SNR)"

bcn_pane="$(tmux split-window -d -v -p 50 -t "$snr_pane" -P -F '#{pane_id}' "$(agent_command "Beacon" "BCN" "docs/agents/BCN.md")")"
tmux select-pane -t "$bcn_pane" -T "$(pane_title BCN)"

# Keep Navigator centered/focused for attach.
tmux select-pane -t "$nav_pane"

if [[ -x "$ROOT/scripts/ocean-bedrock-chrome.sh" ]]; then
  "$ROOT/scripts/ocean-bedrock-chrome.sh" "$SESSION" >/dev/null || true
fi

tmux select-window -t "$SESSION:Crew"
tmux select-pane -t "$nav_pane"

echo "Started rt-nas core crew in tmux session '$SESSION'."
echo "Attach with: tmux attach -t $SESSION"
echo "Layout: centered Navigator cockpit with Anchor/Keel left and Sonar/Beacon right"
echo "Logs: $LOG_DIR"
