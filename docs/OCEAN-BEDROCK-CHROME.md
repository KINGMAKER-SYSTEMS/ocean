# Ocean Bedrock Chrome

Status: active for the `rt-nas-crew` Ocean Bedrock crew.

Purpose: make the crew cockpit feel like a nautical command deck while keeping safety and ops signals readable.

## Layout

The launcher builds a single `Crew` tmux window with Navigator centered:

```txt
┌─────────────┬──────────────────────────┬─────────────┐
│ Anchor      │                          │ Sonar       │
├─────────────┤       Navigator          ├─────────────┤
│ Keel        │       orchestrator       │ Beacon      │
└─────────────┴──────────────────────────┴─────────────┘
```

## Pieces

### Pi TUI theme

Project theme:

```txt
.pi/themes/ocean-bedrock.json
```

Palette: abyss black, sea-glass cyan, lagoon green, warning gold, coral error, shell/sandstone neutrals.

### Pi TUI mode extension

Project extension:

```txt
.pi/extensions/ocean-bedrock-mode.ts
```

What it does:

- auto-activates for rt-nas crew agent panes,
- selects the `ocean-bedrock` theme,
- adds a compact per-agent status chip,
- customizes the streaming/working indicator,
- exposes `/ocean-bedrock` to toggle the mode.

Design note: the first pass used custom headers, large below-editor widgets, and pane-border labels. That was too noisy in narrow panes, so the current version is intentionally clean and compact.

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
scripts/ocean-bedrock-chrome.sh rt-nas-crew
```

What it does:

- paints the tmux status bar with nautical colors,
- keeps pane borders single-line and clean,
- leaves top pane-border labels off,
- normalizes pane titles without drawing over the Pi UI,
- keeps the centered Navigator layout intact.

The crew launcher exports `OCEAN_BEDROCK_MODE=1` for agents and reapplies tmux chrome after launch.

## Apply/reload

For an existing crew:

```bash
cd /root/rt-nas/rt-nas
scripts/ocean-bedrock-chrome.sh rt-nas-crew
```

Then run `/reload` inside each Pi agent pane to hot-load the project theme/extension.

For a fresh crew:

```bash
cd /root/rt-nas/rt-nas
scripts/rt-nas-crew-launch.sh --replace
```

Attach:

```bash
tmux attach -t rt-nas-crew
```

## Notes

- This is visual chrome only; it does not change agent permissions or task execution policy.
- It does not call `pi_messenger work` or autonomous crew execution.
- `.pi` remains mostly ignored, with only the project theme/extension explicitly tracked.
