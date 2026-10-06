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
require("workflow_dispatch:" in source, "manual release-candidate validation is available")
require("pull_request:" not in source, "routine pull requests do not run release packaging")
require('tags: ["v*"]' in source, "stable tag pattern invokes release validation")
require(
    source.index("workflow_dispatch:") < source.index("  push:") < source.index('tags: ["v*"]'),
    "manual dispatch and stable tag are the only events",
)
require("working-directory: platform/ocean-os" in source, "component commands use monorepo component root")
require('SOURCE_ROOT="$GITHUB_WORKSPACE/platform/ocean-os"' in source, "package smoke test reads component licenses")
require("path: platform/ocean-os/release-staging/" in source, "artifact upload path is repository-root-relative")
require(
    "platform/ocean-os/release-staging/ocean-macos-arm64.tar.gz" in source,
    "release asset paths are repository-root-relative",
)
require("RELEASE_TAG_RULESET_ID:" in source and "OCEAN_RELEASE_TAG_RULESET_ID" in source, "tag policy uses provisioned repository variable")
require("curl --fail --location --proto" in source, "public ruleset reads avoid elevated workflow token scope")
require(
    "ruleset.bypass_actors != null && ruleset.bypass_actors.length !== 0" in source,
    "returned bypass metadata fails closed",
)
require("if: github.event_name == 'push'" in source, "only stable-tag pushes enter the publish job")
validate_job = source[source.index("  validate:") : source.index("  publish:")]
publish_job = source[source.index("  publish:") :]
require("contents: read" in validate_job, "validation job remains read-only")
require("packages: write" not in validate_job, "validation has no package-write authority")
require("packages: write" in publish_job, "package write scope is confined to publishing")
require("draft: true" in source, "GitHub Release stays hidden during package publication")
require(
    source.index('test "$reconciled" = "1"') < source.index("Publish verified GitHub Release"),
    "release publication follows registry convergence",
)
require("-F draft=false" in source, "verified release is made public only after registry convergence")

print("Ocean release workflow root/path/security contract: PASS")
