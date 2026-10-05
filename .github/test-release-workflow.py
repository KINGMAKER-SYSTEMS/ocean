#!/usr/bin/env python3
"""Offline contract checks for the root Ocean OS release workflow."""

from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/release.yml"
source = WORKFLOW.read_text()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"release workflow contract failed: {message}")


require(WORKFLOW.is_file(), "GitHub-discoverable root workflow exists")
require("on:\n  pull_request:" in source, "relevant pull requests validate packages")
require("  push:\n    tags: [\"v*\"]" in source, "stable tag pattern invokes publishing gate")
for path in (
    ".github/workflows/release.yml",
    "platform/ocean-os/.github/workflows/release.yml",
    "platform/ocean-os/packaging/**",
    "platform/ocean-os/Cargo.toml",
    "platform/ocean-os/Cargo.lock",
    "platform/ocean-os/crates/**",
):
    require(f'      - "{path}"' in source, f"PR filter includes {path}")

require("working-directory: platform/ocean-os" in source, "component commands use monorepo component root")
require('SOURCE_ROOT="$GITHUB_WORKSPACE/platform/ocean-os"' in source, "package smoke test reads component licenses")
require("path: platform/ocean-os/release-staging/" in source, "artifact upload path is repository-root-relative")
require("platform/ocean-os/release-staging/ocean-macos-arm64.tar.gz" in source, "release asset paths are repository-root-relative")
require('RELEASE_TAG_RULESET_ID: "${{ vars.OCEAN_RELEASE_TAG_RULESET_ID }}"' in source, "tag policy uses provisioned repository variable")
require("curl --fail --location --proto" in source, "public ruleset reads avoid elevated workflow token scope")
require("ruleset.bypass_actors != null && ruleset.bypass_actors.length !== 0" in source, "returned bypass metadata fails closed")
require("if: github.event_name == 'push'" in source, "only tag pushes receive publish job")
require("permissions:\n      contents: read" in source, "validation job is read-only")
require("packages: write" in source, "package write scope is explicit")
require("pull_request.paths" not in source, "release validation is not a root required CI status")
require("draft: true" in source, "GitHub Release stays hidden during package publication")
require(source.index('test "$reconciled" = "1"') < source.index("Publish verified GitHub Release"), "release publication follows registry convergence")
require("-F draft=false" in source, "verified release is made public only after registry convergence")

print("Ocean release workflow root/path/security contract: PASS")
