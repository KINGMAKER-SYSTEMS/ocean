#!/usr/bin/env bash
set -euo pipefail

SESSION="${1:-rt-nas-crew}"

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "tmux session '$SESSION' not found" >&2
  exit 1
fi

# Truecolor nautical shell around the crew cockpit. This is tmux-only chrome;
# pi TUI chrome is handled by .pi/extensions/ocean-barnacle-mode.ts.
tmux set-option -t "$SESSION" status on
tmux set-option -t "$SESSION" status-interval 2
tmux set-option -t "$SESSION" status-position bottom
tmux set-option -t "$SESSION" status-style "bg=#031018,fg=#D9FFF6"
tmux set-option -t "$SESSION" message-style "bg=#123A3A,fg=#D9FFF6,bold"
tmux set-option -t "$SESSION" mode-style "bg=#0B343C,fg=#7EF9E5,bold"
tmux set-option -t "$SESSION" pane-border-status top
tmux set-option -t "$SESSION" pane-border-lines heavy
tmux set-option -t "$SESSION" window-status-separator " #[fg=#314B57]𓇼#[default] "
tmux set-option -t "$SESSION" status-left-length 80
tmux set-option -t "$SESSION" status-right-length 120
tmux set-option -t "$SESSION" status-left "#[fg=#7EF9E5,bold] 𓆉 OCEAN BEDROCK #[fg=#B9A88E]barnacle crew #[fg=#314B57]│ "
tmux set-option -t "$SESSION" status-right "#[fg=#91B7C7]%Y-%m-%d #[fg=#E9C46A]%H:%M UTC #[fg=#314B57]│ #[fg=#7EF9E5]#S "
tmux set-option -t "$SESSION" window-status-format "#[fg=#{@ocean_agent_color}]#{@ocean_agent_icon} #I:#W#[fg=#638190] #{@ocean_agent_lane}"
tmux set-option -t "$SESSION" window-status-current-format "#[bg=#0B343C,fg=#{@ocean_agent_color},bold] #{@ocean_agent_icon} #I:#W #[fg=#D9FFF6]#{@ocean_agent_lane} "

declare -A ICONS=(
  [README]="𓇼"
  [NAV]="🧭"
  [ANK]="⚓"
  [KEL]="⛵"
  [SNR]="◉"
  [BCN]="✦"
)
declare -A COLORS=(
  [README]="#7EF9E5"
  [NAV]="#7EF9E5"
  [ANK]="#E9C46A"
  [KEL]="#8CD867"
  [SNR]="#3AAED8"
  [BCN]="#C084FC"
)
declare -A LANES=(
  [README]="reef-map"
  [NAV]="orchestrator"
  [ANK]="source"
  [KEL]="runtime"
  [SNR]="search"
  [BCN]="health"
)
declare -A TITLES=(
  [README]="𓇼 Reef Map"
  [NAV]="🧭 Navigator-NAV"
  [ANK]="⚓ Anchor-ANK"
  [KEL]="⛵ Keel-KEL"
  [SNR]="◉ Sonar-SNR"
  [BCN]="✦ Beacon-BCN"
)

for win in README NAV ANK KEL SNR BCN; do
  if tmux list-windows -t "$SESSION" -F '#{window_name}' | grep -qx "$win"; then
    target="$SESSION:$win"
    color="${COLORS[$win]}"
    tmux set-option -w -t "$target" @ocean_agent_icon "${ICONS[$win]}"
    tmux set-option -w -t "$target" @ocean_agent_color "$color"
    tmux set-option -w -t "$target" @ocean_agent_lane "${LANES[$win]}"
    tmux set-option -w -t "$target" pane-border-style "fg=#314B57"
    tmux set-option -w -t "$target" pane-active-border-style "fg=$color,bold"
    tmux set-option -w -t "$target" pane-border-format "#[fg=$color,bold] ${TITLES[$win]} #[fg=#B9A88E]𓇼 barnacles #[fg=#314B57]#(printf '─%.0s' {1..8})"
    tmux select-pane -t "$target" -T "${TITLES[$win]}"
  fi
done

# Paint the README/reaf-map pane with a non-destructive splash if it is just a shell.
if tmux list-windows -t "$SESSION" -F '#{window_name}' | grep -qx README; then
  pane_cmd="$(tmux display-message -p -t "$SESSION:README.1" '#{pane_current_command}' 2>/dev/null || true)"
  if [[ "$pane_cmd" == "bash" || "$pane_cmd" == "zsh" || "$pane_cmd" == "sh" ]]; then
    tmux send-keys -t "$SESSION:README.1" "clear; printf '\033[38;2;126;249;229m%s\033[0m\n' '        𓆉  OCEAN BEDROCK CREW  𓆉'; printf '\033[38;2;185;168;142m%s\033[0m\n' '  ╭────𓇼──── barnacle-encrusted reef deck ────𓇼────╮'; printf '\033[38;2;217;255;246m%s\033[0m\n' '  │ NAV 🧭  ANK ⚓  KEL ⛵  SNR ◉  BCN ✦          │'; printf '\033[38;2;185;168;142m%s\033[0m\n' '  ╰──── shell sealed · secrets below deck · tide high ────╯'; echo; tmux list-windows -t $SESSION" C-m
  fi
fi

echo "Ocean Barnacle chrome applied to tmux session '$SESSION'."
