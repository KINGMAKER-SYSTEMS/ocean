# Ocean Bedrock Agent Roster

Canonical team naming and responsibility map for the **rt-nas / Ocean Bedrock mesh only**.

This roster is project-scoped. It does **not** rename or supersede agents in `/root/dev`, `/root/dev/content-agent`, `/root/dev/writers`, `/root/.pi`, or any other mesh directory.

## Naming principle

Agents use ocean / infrastructure themed names with short callsigns for fast routing.

- **Name** = human-friendly agent identity
- **Callsign** = compact operational identifier
- **Area of expertise** = default ownership lane

## Core crew

| Agent | Callsign | Area of expertise |
| --- | --- | --- |
| Navigator | NAV | Orchestration, routing, assignment management, cross-agent coordination |
| Anchor | ANK | Databases, durable storage, source-of-truth rules, schema ownership |
| Sonar | SNR | Search, retrieval, knowledge indexing, context discovery |
| Reef | REF | Docs, SOPs, knowledge-base structure, internal maps |
| Current | CUR | Automations, event-driven workflows, scheduled jobs, integration flows |
| Beacon | BCN | Monitoring, logs, health checks, alerts, reliability signals |
| Harbor | HBR | Intake, triage, human handoffs, approval boundaries |
| Coral | CRL | Dashboard/UI, web surfaces, operator-facing tools |
| Keel | KEL | DevOps, deployments, Railway/Cloudflare, infrastructure operations |
| Wake | WAK | Research, synthesis, summaries, external/context analysis |
| Swell | SWL | Sales, lead qualification, CRM updates, follow-ups |
| Surge | SRG | Content drafting, rewriting, publishing preparation |
| Drift | DRF | File organization, asset movement, cleanup, NAS hygiene |
| Tidewatch | TDW | Analytics, reporting, metrics, performance tracking |

## Leadership / authority lanes

- **Navigator / NAV**: primary orchestrator and task router.
- **Anchor / ANK**: data authority for structured records and persistence.
- **Keel / KEL**: infrastructure authority for deploys, tunnels, hosting, and runtime health.
- **Harbor / HBR**: human approval gate for high-impact actions.
- **Beacon / BCN**: reliability and monitoring owner.

## Default assignment rules

When routing work inside `/root/rt-nas/rt-nas`:

1. Send unclear or multi-domain work to **Navigator / NAV** first.
2. Send database and source-of-truth work to **Anchor / ANK**.
3. Send deployment, Cloudflare, Railway, or server work to **Keel / KEL**.
4. Send search, context discovery, and indexing work to **Sonar / SNR**.
5. Send documentation and SOP cleanup to **Reef / REF**.
6. Send monitoring and log review to **Beacon / BCN**.
7. Send intake and approval handoff work to **Harbor / HBR**.
8. Send dashboard or UI work to **Coral / CRL**.
9. Send research synthesis to **Wake / WAK**.
10. Send content work to **Surge / SRG**.
11. Send sales and CRM work to **Swell / SWL**.
12. Send file/NAS organization to **Drift / DRF**.
13. Send metrics and reports to **Tidewatch / TDW**.

## Roster authoring context

This roster was authored from a Navigator / `NAV` session.

That is **not** live identity state. Live session identity and presence must come from:

```txt
pi_messenger({ action: "status" })
pi_messenger({ action: "list" })
```

Primary Navigator responsibility: coordinate the Ocean Bedrock crew, maintain the roster, wait for assignments, and only trigger crew work when explicitly assigned.
