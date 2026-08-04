# Monorepo migration

## Imported repositories

| Monorepo path | Source repository | Source branch | Imported commit |
| --- | --- | --- | --- |
| `platform/ocean-os` | `Risingtides-dev/ocean-os` | `main` | `105d5dbcc4db839919838f31c8071dbabadb7f6f` |
| `apps/ocean-surface` | `Risingtides-dev/ocean-surface` | `main` | `bbe75c2cb95175c1d5974ce3117bf79d7ad3a604` |
| `packages/ocean-agents` | `Risingtides-dev/ocean-agents` | `main` | `f21d05f610d05aaf024f7e272719ea66bd206142` |
| `services/ocean-bedrock` | `Risingtides-dev/ocean-bedrock` | `master` | `22d19b398247dce59785f1422adb3e13e306a2c4` |
| `org/risingtides-agents` | `Risingtides-dev/risingtides-agents` | `main` | `2d9594e82146acdfe8d4004b21cd88223ae806a3` |

Each project was imported with `git subtree` **without squashing**, so its existing
commit history remains available in this repository.

## Transition policy

The source repositories remain active while the team validates this workspace.
During that period:

1. Decide which repository is authoritative before beginning a change.
2. Avoid making the same change independently in both locations.
3. Land cross-project changes in this monorepo when atomic review is valuable.
4. Do not archive a source repository until its CI, release jobs, deployments,
   secrets, branch protections, and team permissions have replacements here.
5. Never split private `org/` or `services/` material into a public destination.

## Pulling a source update

Run the relevant `git subtree pull` from the monorepo root. For example:

```bash
git subtree pull \
  --prefix=platform/ocean-os \
  https://github.com/Risingtides-dev/ocean-os.git main
```

Review the resulting merge and run the owning project's validation before push.
Do not use `--squash`; preserving ancestry makes future synchronization and
forensics clearer.

## Local validation entry points

```bash
# Ocean OS
(cd platform/ocean-os && cargo xtask ci)

# Ocean Surface
(cd apps/ocean-surface && cargo check --workspace)

# Ocean Agents
(cd packages/ocean-agents && make assistants-check)

# Rising Tides Agents
(cd org/risingtides-agents && make assistants-check)

# Bedrock
(cd services/ocean-bedrock && npm ci && npm test)
```

The nearest project `AGENTS.md` remains authoritative if these commands change.
