# Ocean Barnacle Chrome

Status: active for the `rt-nas-crew` Ocean Bedrock crew.

Purpose: make the crew cockpit feel like a nautical command deck while keeping the safety/ops signal readable.

## Pieces

### Pi TUI theme

Project theme:

```txt
.pi/themes/ocean-barnacle.json
```

Palette: abyss black, sea-glass cyan, lagoon green, warning gold, coral error, barnacle/shell neutrals.

### Pi TUI mode extension

Project extension:

```txt
.pi/extensions/ocean-barnacle-mode.ts
```

What it does:

- auto-activates for rt-nas crew agent panes,
- selects the `ocean-barnacle` theme,
- replaces the Pi header with an Ocean Bedrock reef banner,
- adds an agent-aware footer,
- adds a below-editor reef status widget,
- customizes the streaming/working indicator,
- exposes `/ocean-barnacle` to toggle the mode.

Agent colors:

| Agent | Callsign | Color lane |
|---|---:|---|
| Navigator | NAV | sea-glass cyan |
| Anchor | ANK | anchor gold |
| Keel | KEL | kelp green |
| Sonar | SNR | tide blue |
| Beacon | BCN | beacon purple |

### tmux shell chrome

Script:

```bash
scripts/ocean-barnacle-chrome.sh rt-nas-crew
```

What it does:

- turns on top pane border labels,
- uses heavy pane borders,
- paints the tmux status bar with nautical colors,
- assigns per-agent window/pane icon, lane, and active border color,
- paints the README reef-map pane with a splash banner.

The crew launcher now exports `OCEAN_BARNACLE_MODE=1` for agents and reapplies tmux chrome after launch.

## Apply/reload

For an existing crew:

```bash
cd /root/rt-nas/rt-nas
scripts/ocean-barnacle-chrome.sh rt-nas-crew
```

Then run `/reload` inside each Pi agent pane to hot-load the project theme/extension.

For a new crew:

```bash
cd /root/rt-nas/rt-nas
scripts/rt-nas-crew-launch.sh
```

## Notes

- This is visual chrome only; it does not change agent permissions or task execution policy.
- It does not call `pi_messenger work` or autonomous crew execution.
- `.pi` remains mostly ignored, with only the project theme/extension explicitly tracked.
