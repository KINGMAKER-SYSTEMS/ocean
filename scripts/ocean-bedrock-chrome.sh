#!/usr/bin/env bash
set -euo pipefail

SESSION="${1:-rt-nas-crew}"

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "tmux session '$SESSION' not found" >&2
  exit 1
fi

# Clean Ocean Bedrock shell chrome. Keep it readable: no top pane labels,
# no giant art in live panes, no layout mutation.
tmux set-option -t "$SESSION" status on
tmux set-option -t "$SESSION" status-interval 5
tmux set-option -t "$SESSION" status-position bottom
tmux set-option -t "$SESSION" status-style "bg=#031018,fg=#D9FFF6"
tmux set-option -t "$SESSION" message-style "bg=#123A3A,fg=#D9FFF6,bold"
tmux set-option -t "$SESSION" mode-style "bg=#0B343C,fg=#7EF9E5,bold"
tmux set-option -t "$SESSION" pane-border-status off
tmux set-option -t "$SESSION" pane-border-lines single
tmux set-option -t "$SESSION" pane-border-style "fg=#314B57"
tmux set-option -t "$SESSION" pane-active-border-style "fg=#7EF9E5"
tmux set-option -t "$SESSION" window-status-separator " #[fg=#314B57]·#[default] "
tmux set-option -t "$SESSION" status-left-length 80
tmux set-option -t "$SESSION" status-right-length 120
tmux set-option -t "$SESSION" status-left "#[fg=#7EF9E5,bold] 𓆉 OCEAN #[fg=#B9A88E]rt-nas #[fg=#314B57]│ "
tmux set-option -t "$SESSION" status-right "#[fg=#91B7C7]%Y-%m-%d #[fg=#E9C46A]%H:%M #[fg=#314B57]│ #[fg=#7EF9E5]#S "
tmux set-option -t "$SESSION" window-status-format "#[fg=#638190]#I:#W"
tmux set-option -t "$SESSION" window-status-current-format "#[bg=#0B343C,fg=#7EF9E5,bold] #I:#W "

# Normalize pane titles without rendering them as border clutter.
declare -A TITLES=(
  [NAV]="Navigator-NAV"
  [ANK]="Anchor-ANK"
  [KEL]="Keel-KEL"
  [SNR]="Sonar-SNR"
  [BCN]="Beacon-BCN"
)

for key in NAV ANK KEL SNR BCN; do
  while IFS='|' read -r pane_id title; do
    [[ -z "${pane_id:-}" ]] && continue
    tmux select-pane -t "$pane_id" -T "${TITLES[$key]}" || true
  done < <(tmux list-panes -s -t "$SESSION" -F '#{pane_id}|#{pane_title}' | awk -F'|' -v key="$key" '$0 ~ key {print $1 "|" $2}')
done

echo "Clean Ocean Bedrock chrome applied to tmux session '$SESSION'."
