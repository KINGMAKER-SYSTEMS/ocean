# hub

Manifest-driven command resolution and courier dispatch.

```bash
python3 router.py list
python3 router.py resolve /ship
python3 router.py run --dry-run /ship --channel C0123ABCD
python3 router.py run /ship --channel C0123ABCD
```

- Discovers every sibling `courier.toml`; adding a package needs no router edit.
- Router flags go before the slash command. Arguments after the slash are passed
  to the selected package.
- Deterministic packages execute their harness directly.
- Agentic packages request a scoped Ocean daemon turn and use the package
  directory as `cwd`, allowing normal `AGENTS.md` and `CLAUDE.md` loading.
- A supplied session key is folded into a stable daemon session ID.
- Transient daemon failures are retried with bounded backoff.

A deployment-specific client or intake adapter may invoke this router. The
public repository intentionally does not ship Slack app identity or destination
configuration. See [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
