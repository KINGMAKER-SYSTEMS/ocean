# Ocean Public Exposure and License-Provenance Audit — 2026-07-19

Status: completed evidence baseline; action register open.

Classification: private Rising Tides security and release-planning record. Do not
copy this report into a public issue or public repository.

## Decision summary

The three public repositories can continue toward an explicitly licensed
developer preview, but they are not ready for a blanket repository-wide license
today.

- `ocean-bedrock` is now private and optional. Its prior public disclosure must
  be treated as permanent.
- No active credential was confirmed in `ocean-os` or `ocean-agents` reachable
  history.
- One organization-owned Google Maps browser key was still bundled in current
  `ocean-surface` source. It was removed from public `main` in commit `05f5283`.
  Cloud-side restriction or revocation remains required before GitHub alert 1
  can be closed.
- Public history rewriting is not justified by current evidence. Public forks,
  pull-request refs, and external clones already retain older objects; rewrite
  only if a later red finding requires coordinated purge.
- `ocean-agents` is the main licensing-boundary risk because public runtime
  profiles/tooling are mixed with Rising Tides production assistants, couriers,
  campaign assets, and rendered output.
- `ocean-surface` needs an asset-provenance pass and an included Poppins license
  before repository-wide licensing.
- `ocean-os` has third-party `NOTICE.md` and crate-scoped licenses but no root
  license. `NOTICE.md` currently refers to “the project's license,” which does
  not yet exist at repository root.

## Scope and method

Evidence was captured on 2026-07-19 from GitHub and local checkouts for:

- `Risingtides-dev/ocean-os`
- `Risingtides-dev/ocean-agents`
- `Risingtides-dev/ocean-surface`
- `Risingtides-dev/ocean-bedrock`

For each repository, the audit:

1. created a fresh Git mirror;
2. fetched branches, tags, GitHub pull-request head/merge refs, and reachable
   public fork heads where forks existed;
3. scanned every reachable ref with checksum-verified Gitleaks `v8.30.1`, full
   redaction, recursive decoding, archive depth 2, and a 10 MiB per-blob cap;
4. independently inventoried every reachable blob size;
5. inspected GitHub visibility, security settings, releases, release assets,
   Actions artifacts, branches, tags, pull requests, Pages, and forks;
6. inventoried current and historical paths, root trust/license files, binary
   assets, and obvious internal-factory candidates; and
7. manually triaged scanner findings without recording any secret value.

No reachable blob exceeded 10 MiB, so the scanner size cap skipped no reachable
blob. The scan covered 6,263 reachable blob objects totaling about 500 MiB
before Git object deduplication. The report does not claim to cover unreachable
objects already pruned by GitHub, private external clones, screenshots, copied
chat transcripts outside Git, or third-party caches.

## Reachable-history inventory

| Repository | Visibility | Mirrored refs | Reachable commits | Contributors | Historical paths | Current paths | Deleted/non-default paths |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `ocean-os` | public | 633 | 1,162 | 10 | 716 | 562 | 154 |
| `ocean-agents` | public | 82 | 104 | 9 | 228 | 210 | 18 |
| `ocean-surface` | public | 214 | 592 | 8 | 819 | 346 | 473 |
| `ocean-bedrock` | private | 17 | 92 | 3 | 170 | 162 | 8 |

“Mirrored refs” includes duplicate reachability through branch, PR, remote, and
fork refs. It is an exposure-surface count, not a count of independent lines of
development.

## GitHub exposure surface

At capture time:

| Repository | Branches | Tags | Open PRs | Forks | Releases/assets | Actions artifacts | Pages |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `ocean-os` | 17 | 0 | 1 | 1 | 0 / 0 | 0 | off |
| `ocean-agents` | 4 | 0 | 1 | 0 | 0 / 0 | 0 | off |
| `ocean-surface` | 6 | 2 | 2 | 1 | 0 / 0 | 0 | off |
| `ocean-bedrock` | 1 | 0 | 0 | 0 | 0 / 0 | 0 | off |

Public forks retained relevant prior history:

- `agentsworlddotorg/ocean-os`
- `ecfromthedc/ocean-surface`

That fact materially lowers the value of an amber-only history rewrite. It does
not block a red purge if one becomes necessary, but a purge cannot promise
recall from those forks or other clones.

## Credential findings and disposition

### RED-1 — public Surface Maps key

Gitleaks found two reachable source-history occurrences of one Google API key
value. GitHub secret scanning independently records alert 1 as open, publicly
leaked, and validity unknown. GitHub also records a historical location in a
checked-in agent transcript artifact.

Current containment:

- commit `05f5283` removed the organization-owned default from
  `crates/ocean-surface-proxy/src/main.rs`;
- maps now enable only from an explicit non-empty `GOOGLE_MAPS_API_KEY`;
- absence leaves the existing map-unavailable notice;
- current `origin/main` has zero Google-key-shaped literals;
- `cargo fmt --check`, `cargo check -p ocean-surface-proxy`, current-tree
  Gitleaks GCP scan, and `git diff --check` passed.

Remaining operator/cloud action:

1. identify the key by the GitHub alert in the authorized GCP project;
2. revoke it, or prove and preserve strict HTTP-referrer and Maps-API-only
   restrictions with bounded quota/billing controls;
3. provide a replacement deployment key through the secret-managed runtime
   environment if production maps remain desired; and
4. resolve GitHub alert 1 as revoked only after cloud verification.

Do not resolve the alert as “used in tests” or “false positive.” A browser key
is intentionally client-visible, but an organization-owned default does not
belong in a public reusable product.

History rewrite decision: no rewrite at this gate. The value is already in a
public fork and many PR refs. Revoke/restrict the credential; do not destabilize
all repository hashes unless cloud evidence shows a broader red incident.

### `ocean-os` — four scanner findings

The four reachable generic-key findings are synthetic test fixtures or prose:

- an `ocean-call` token test fixture;
- a persistent-room registration-key assertion;
- an extension manifest test fixture; and
- another historical occurrence of the token test fixture.

No production credential was identified. Classification: green false positives.
A future `.gitleaks.toml` may suppress only exact fixture fingerprints or
narrowly annotated fixture lines; do not add broad path exclusions.

### `ocean-agents` — zero findings

Gitleaks reported zero reachable findings. GitHub secret scanning and push
protection were disabled at capture and were enabled during this audit. This is
now the minimum public-repository posture.

### `ocean-bedrock` — two revoked-token evidence findings

Both findings are in private workflow evidence documents and label their value
as `tokenRevoked`; the recorded response is HTTP 403. They appear to be evidence
of completed revocation rather than active credentials.

Classification: amber/closed pending an operator confirmation that the backing
tokens remain revoked. Do not reproduce the values. Because Bedrock is now
private and the evidence already records revocation, no public history rewrite
is recommended.

## Current licensing and provenance boundary

### `ocean-os`

Current root trust files:

- `NOTICE.md`

Missing at root:

- `LICENSE` or dual-license files;
- `SECURITY.md`;
- `SUPPORT.md`;
- `CONTRIBUTING.md`;
- `CODE_OF_CONDUCT.md`; and
- `GOVERNANCE.md`.

`NOTICE.md` preserves MIT attribution for the `pi-agent`/`pi-ai` lineage.
`crates/ocean-minimizer` and `crates/ocean-walker` also carry crate-scoped MIT
license/notice files. These notices must be retained and reviewed before adding
a root license. Do not infer that a future root license retroactively grants
rights over unrelated historical artifacts.

Current internal-factory candidates include root `HANDOFF.md`, `events.md`, and
`docs/.agentarchive/**`. These are not credential findings. Decide whether they
serve transparent public development or expose unnecessary company process;
move only current amber material and leave history unless a red trigger appears.

### `ocean-agents`

There is no root license or trust/community baseline. The repository combines:

- public surface profiles and router/composition tooling;
- provider-agnostic package conventions;
- Rising Tides production assistants and couriers; and
- campaign anchors/rendered media under `assistants/content-agent/**`.

Current binary media includes 10 images and 4 rendered MP4 files, all concentrated
under the production content-agent package. No per-asset provenance manifest was
found. The following package families have explicit Rising Tides references and
must be classified before licensing:

- `assistants/content-agent`
- `assistants/voice-intake`
- `assistants/prompt-lab`
- `assistants/creator-research`
- `assistants/yt-prompt-bible`
- `assistants/bonzai`
- `assistants/bridge`
- `couriers/uiau`
- `couriers/yt-clipper`

Required boundary: define a public allowlist of reusable house profiles, tooling,
schemas, examples, and generic couriers. Move Rising Tides production behavior,
brand/campaign assets, customer-specific prompts, and rendered output to a
private package repository before adding a root license.

### `ocean-surface`

There is no root license or trust/community baseline. The current tree contains
85 binary/design assets: 58 PNG, 22 SVG, 4 WOFF2, and 1 ICO. Most are concentrated
in Tauri/legacy GUI assets; four Poppins font files are under `public/fonts/`.
No corresponding root or colocated Poppins license was found.

Required actions before licensing:

1. prove source and permitted redistribution for each retained font, icon, logo,
   screenshot, and generated asset family;
2. add the correct Poppins license and attribution if those exact files are
   retained;
3. remove development screenshots and test renders from the distribution path
   unless intentionally documented and provenance-cleared;
4. distinguish Ocean-owned brand assets from third-party product screenshots;
   and
5. preserve license texts in release bundles.

### `ocean-bedrock`

Bedrock remains private. It requires internal copyright/provenance hygiene but
is not a blocker to public Ocean use or public repository licensing. Any future
public Bedrock protocol/SDK must be published as a deliberately scoped artifact,
not by reopening the private repository.

## Action register

### P0 — containment

- [x] Make Bedrock private and verify authorized clone/access.
- [x] Remove dead public GitHub links and document Bedrock as optional.
- [x] Retire the obsolete public Bedrock coworker-onboarding skill.
- [x] Remove the bundled Surface Maps key from current public source.
- [x] Enable GitHub secret scanning and push protection on `ocean-agents`.
- [ ] Revoke or verify strict restrictions for the alerted Maps key in GCP.
- [ ] Resolve GitHub Surface alert 1 only after cloud verification.
- [ ] Confirm both Bedrock evidence tokens remain revoked.

### P1 — separate licensable product from private factory

- [ ] Create an `ocean-agents` public-package allowlist.
- [ ] Move production assistant/courier packages and content-agent media/output
      to a private package location while preserving public generic tooling.
- [ ] Classify `HANDOFF.md`, `events.md`, archived plans, and operational ledgers
      in each public repository as public-development evidence or private
      company process.
- [ ] Add an asset provenance manifest to `ocean-surface` and close every unknown
      asset family.
- [ ] Add the correct Poppins redistribution license or remove the fonts.

### P2 — licensing and trust baseline

- [ ] Record the license decision, scope, copyright holder, and effective tagged
      release; obtain legal/operator approval.
- [ ] Reconcile the root license with `ocean-os/NOTICE.md` and crate-scoped MIT
      notices.
- [ ] Add `LICENSE`, `NOTICE`, SPDX policy, `SECURITY.md`, `SUPPORT.md`,
      contribution rules, and governance/community files to each public repo as
      applicable.
- [ ] Enable and verify dependency/security automation appropriate to each repo.

### P3 — public distribution and onboarding

- [ ] Produce signed/versioned release artifacts; no repository currently has a
      GitHub release asset.
- [ ] Build `ocean init`, `ocean doctor`, and a deterministic local-only start
      path that does not require Bedrock.
- [ ] Validate the five-minute quickstart on a clean machine/account.
- [ ] Run a small external beta before broad promotion.

## History-rewrite rule

Do not rewrite for amber internal material. Rewrite only after a confirmed red
finding involving active credentials, regulated/customer data, contractually
confidential third-party material, or content Rising Tides has no right to
publish.

A red rewrite requires, in order: revoke/contain, freeze pushes, preserve a
private mirror, define exact path/text filters, run `git filter-repo`, verify all
refs, force-push branches/tags, require contributor reclones, coordinate public
fork owners where possible, and request GitHub cache cleanup. Even then, prior
disclosure must be treated as permanent.

## Evidence handling

Raw Gitleaks JSON reports and mirrors were kept only in local temporary audit
paths. They are fully redacted but were not committed. This report intentionally
contains no key, token, private URL, raw contributor email, or credential
fingerprint.
