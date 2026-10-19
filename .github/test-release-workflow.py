#!/usr/bin/env python3
"""Offline contract checks for the root Ocean OS release workflow."""

from pathlib import Path
import json
import hashlib
import os
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/release.yml"
source = WORKFLOW.read_text()
package = json.loads((ROOT / "platform/ocean-os/packaging/npm/package.json").read_text())


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"release workflow contract failed: {message}")


require(WORKFLOW.is_file(), "GitHub-discoverable root workflow exists")
require(package["name"] == "@kingmaker-systems/ocean", "release package belongs to the canonical organization")
require(package["repository"] == {
    "type": "git",
    "url": "git+https://github.com/KINGMAKER-SYSTEMS/ocean.git",
    "directory": "platform/ocean-os/packaging/npm",
}, "package links to its canonical monorepo component")
require('scope: "@kingmaker-systems"' in source, "registry authentication uses the organization scope")
require(source.count('package="@kingmaker-systems/ocean"') == 2, "publication and latest-tag reconciliation target the same org package")
require("@risingtides-dev" not in source, "active publisher cannot target the personal package")
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
require('gh release view "$GITHUB_REF_NAME" --repo "$GITHUB_REPOSITORY"' in publish_job,
        "checkout-free release lookup names its repository explicitly")
download_step = publish_job.split("      - name: Download and verify exact validated artifact\n", 1)[1].split("\n      - name:", 1)[0]
require("working-directory: ${{ github.workspace }}" in download_step,
        "checkout-free artifact download starts in the existing workspace")
download_script = "\n".join(line[10:] for line in download_step.split("        run: |\n", 1)[1].splitlines())
# Execute the actual download shell in an empty workspace. Only the network
# command is replaced; digest verification and staging use the real tools.
with tempfile.TemporaryDirectory() as temp:
    fixture = Path(temp)
    payload = fixture / "payload"
    payload.mkdir()
    (payload / "proof.txt").write_text("validated artifact\n")
    archive = Path(shutil.make_archive(str(fixture / "artifact"), "zip", payload))
    bin_dir = fixture / "bin"
    bin_dir.mkdir()
    fake_gh = bin_dir / "gh"
    fake_gh.write_text('#!/bin/sh\ncat "$FIXTURE_ARCHIVE"\n')
    fake_gh.chmod(0o755)
    runner_temp = fixture / "runner"
    runner_temp.mkdir()
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}",
           "FIXTURE_ARCHIVE": str(archive), "RUNNER_TEMP": str(runner_temp),
           "GITHUB_REPOSITORY": "fixture/ocean", "ARTIFACT_ID": "1",
           "EXPECTED_DIGEST": hashlib.sha256(archive.read_bytes()).hexdigest()}
    workspace = fixture / "workspace"
    workspace.mkdir()
    subprocess.run(["bash", "-c", download_script], cwd=workspace, env=env, check=True)
    require((workspace / "platform/ocean-os/release-staging/proof.txt").read_text() == "validated artifact\n",
            "verified artifact stages without a source checkout")
    bad_workspace = fixture / "bad-workspace"
    bad_workspace.mkdir()
    result = subprocess.run(["bash", "-c", download_script], cwd=bad_workspace,
                            env={**env, "EXPECTED_DIGEST": "0" * 64})
    require(result.returncode != 0 and not (bad_workspace / "platform").exists(),
            "digest mismatch fails before component staging")
require("draft: true" in source, "GitHub Release stays hidden during package publication")
require(
    source.index('test "$reconciled" = "1"') < source.index("Publish verified GitHub Release"),
    "release publication follows registry convergence",
)
require("-F draft=false" in source, "verified release is made public only after registry convergence")

print("Ocean release workflow root/path/security contract: PASS")
