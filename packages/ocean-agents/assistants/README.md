# Assistants

Assistants are editable, provider-neutral behavior packages loaded by Ocean OS
according to the active client surface.

## Layout

```text
assistants/
├── _shared/system.md          # invariants shared by every surface
├── _base/<SURFACE>/*.md       # authored surface sources
├── <SURFACE>/system.md        # generated profile Ocean loads
├── bonzai/                    # public named-specialist reference
└── tools/compose_profile.py   # deterministic composer
```

Ocean maps `client_type` to one of ten public surfaces: ACP, browser, CLI,
canvas, GUI, mobile, Slack, TUI, voice, or web. The daemon loads one generated
`assistants/<SURFACE>/system.md` file from `OCEAN_ASSISTANTS_DIR` or its normal
assistants configuration root.

## Composition model

The runtime currently loads one published profile per surface. This repository
keeps shared rules DRY by composing:

1. `_shared/system.md`;
2. `_base/<SURFACE>/` files in the documented order; and
3. optional named-specialist overrides.

The output is `assistants/<SURFACE>/system.md`.

### Authoring flow

```bash
# Edit _shared or _base sources, then:
python3 assistants/tools/compose_profile.py <SURFACE> --write
make assistants-check
```

Commit source and generated output together. CI rejects drift, missing output,
and empty published profiles.

## Public registry

| Surface | Package | Purpose |
| --- | --- | --- |
| `[ACP]` | ACP house profile | Native editor agent panel |
| `[BRWSR]` | Browser house profile | Docked browser control surface |
| `[CLI]` | CLI house profile | Scriptable one-shot interaction |
| `[CNVS]` | Canvas house profile | Spatial, durable visual work |
| `[GUI]` | GUI house profile | Native desktop surface |
| `[MOBL]` | Mobile house profile | Glanceable on-the-move interaction |
| `[SLACK]` | Slack house profile | Thread/channel-aware communication |
| `[TUI]` | TUI house profile | Terminal cockpit behavior |
| `[VOX]` | Voice house profile | Spoken-clean, hands-free interaction |
| `[WEB]` | Web house profile | Rich component-capable web surface |
| `[BONZAI]` | `bonzai` | Generic Git branch and worktree hygiene |

## Adding a surface or specialist

A public package must be reusable without organization-specific accounts,
knowledge, destinations, media, or operating procedures. Add authored sources,
compose the published profile, update the indexes, and run the repository
checks.
