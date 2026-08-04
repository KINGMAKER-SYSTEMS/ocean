# Assistant package contract

## Purpose

Own provider-agnostic surface specialists and the composed house profiles loaded by Ocean OS.

## Ownership

- `README.md` owns assistant and surface-profile orientation and composition workflow.
- `_shared/` owns cross-surface identity shared by composed profiles.
- `_base/` owns house profile sources.
- Each named assistant directory owns its profile, protocol, harness, and local devlog.

## Local Contracts

- Generated surface profiles are composed artifacts; follow `README.md` and do not hand-edit generated output.
- Keep provider-specific transport and runtime authority outside assistant profiles.
- Read the assistant's child `AGENTS.md` before editing its files.

## Work Guidance

- Keep specialists narrow and surface-aware.
- Validate composed profiles with the repository's existing composition checks.

## Verification

- Run the composition or narrow package check documented by the touched assistant.

## Child devlog Index

- `bonzai/` — branch and repository hygiene specialist → `bonzai/AGENTS.md`
