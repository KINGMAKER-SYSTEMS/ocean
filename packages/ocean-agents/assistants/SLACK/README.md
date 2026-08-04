# `[SLACK]` surface profile

`system.md` is the composed Slack profile Ocean loads for a turn whose
`client_type` resolves to `surface-slack`.

Do not edit the generated file directly. Edit sources under
`assistants/_base/SLACK/` or `assistants/_shared/system.md`, then run:

```bash
python3 assistants/tools/compose_profile.py SLACK --write
make assistants-check
```

Ocean resolves the profile root from `OCEAN_ASSISTANTS_DIR`, falling back to its
configured assistants directory. Without this profile, the daemon uses its
compiled generic surface behavior.

Client intake, session creation, SSE output, and daemon request contracts are
documented in [`../../docs/DAEMON_INTERACTION.md`](../../docs/DAEMON_INTERACTION.md).
Deployment-specific Slack app configuration does not belong in this public
package repository.
