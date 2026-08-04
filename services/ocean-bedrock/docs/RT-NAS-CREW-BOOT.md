# rt-nas Crew Boot

Project-scoped boot notes for the **rt-nas / Ocean Bedrock** agent crew.

## Scope boundary

This boot procedure applies only to:

```txt
/root/rt-nas/rt-nas
```

Do not use this roster or boot process to rename, reset, or control agents in:

- `/root/dev`
- `/root/dev/content-agent`
- `/root/dev/writers`
- `/root/.pi`
- any other mesh directory

## Source of truth

Agent names, callsigns, and expertise lanes are defined in:

- Human-readable: `docs/OCEAN-BEDROCK-AGENT-ROSTER.md`
- Machine-readable: `docs/ocean-bedrock-agent-roster.json`

## Boot principle

Each agent should be started from `/root/rt-nas/rt-nas` and should treat this repository as its only project scope unless explicitly instructed otherwise.

Recommended agent identity format:

```txt
<Name> — <CALLSIGN>
```

Example:

```txt
Navigator — NAV
```

## Activation command

Launch the initial rt-nas core crew with:

```bash
cd /root/rt-nas/rt-nas
scripts/rt-nas-crew-launch.sh
```

Attach to the running crew with:

```bash
tmux attach -t rt-nas-crew
```

Local logs are written to:

```txt
.pi/crew-logs/
```

## Initial priority agents

Start with a small core crew before launching the full roster:

1. **Navigator / NAV** — orchestration and routing
2. **Anchor / ANK** — database/storage/source-of-truth ownership
3. **Keel / KEL** — runtime/deployment/infrastructure ownership
4. **Sonar / SNR** — search/retrieval/context discovery
5. **Beacon / BCN** — monitoring/logs/health checks

Add other agents only when there is a clear task lane for them.

## Starter instruction files

Per-agent starter instructions live in:

```txt
docs/agents/
```

Current starter files:

- `docs/agents/NAV.md`
- `docs/agents/ANK.md`
- `docs/agents/KEL.md`
- `docs/agents/SNR.md`
- `docs/agents/BCN.md`

## Safe startup checklist

Before assigning work to a crew agent:

1. Confirm current directory is `/root/rt-nas/rt-nas`.
2. Confirm the task belongs to Ocean Bedrock / rt-nas.
3. Confirm the agent's callsign and expertise lane.
4. Avoid modifying files outside this repository unless the human explicitly asks.
5. For high-impact actions, route through **Harbor / HBR** for human approval.

High-impact actions include:

- external publishing
- deleting important data
- production deployments with meaningful risk
- credential or secret changes
- large-scale outbound messaging

## First recommended work queue

Initial Ocean Bedrock tasks should focus on stability and clarity:

1. Service health/status check
2. Database/source registry review
3. Ingest pipeline review
4. Documentation cleanup
5. Deployment/runtime check
6. Monitoring/logging baseline

## pi-messenger note

If `pi_messenger({ action: "join" })` fails due to stale or cross-directory state, do not treat that as permission to touch other project meshes.

Only inspect or repair messenger files under:

```txt
/root/rt-nas/rt-nas/.pi/
```

If a repair would require touching another directory, ask the human first.
