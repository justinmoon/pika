from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = REPO_ROOT / "scripts" / "forge-github-ci-shim.py"


def git(cwd: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=True,
        text=True,
        capture_output=True,
    )
    return completed.stdout.strip()


class ForgeGithubCiShimTests(unittest.TestCase):
    def test_branch_selection_uses_branch_head_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            git(repo, "init")
            git(repo, "config", "user.name", "Test User")
            git(repo, "config", "user.email", "test@example.com")
            (repo / "ci").mkdir()
            (repo / "docs").mkdir()
            (repo / "README.md").write_text("base\n", encoding="utf-8")
            (repo / "docs" / "guide.md").write_text("docs\n", encoding="utf-8")
            (repo / "ci" / "forge-lanes.toml").write_text(
                """
version = 1
nightly_schedule_utc = "08:00"

[[branch.lanes]]
id = "docs"
title = "docs"
entrypoint = "printf docs"
command = ["python3", "-c", "print('docs')"]
paths = ["docs/**"]

[[nightly.lanes]]
id = "nightly"
title = "nightly"
entrypoint = "printf nightly"
command = ["python3", "-c", "print('nightly')"]
""".strip()
                + "\n",
                encoding="utf-8",
            )
            git(repo, "add", "README.md", "docs/guide.md", "ci/forge-lanes.toml")
            git(repo, "commit", "-m", "base")
            base = git(repo, "rev-parse", "HEAD")

            (repo / "ci" / "forge-lanes.toml").write_text(
                """
version = 1
nightly_schedule_utc = "08:00"

[[branch.lanes]]
id = "docs"
title = "docs"
entrypoint = "printf docs"
command = ["python3", "-c", "print('docs')"]
paths = ["docs/**"]

[[branch.lanes]]
id = "manifest_only"
title = "manifest-only"
entrypoint = "printf manifest-only"
command = ["python3", "-c", "print('manifest-only')"]
paths = ["ci/forge-lanes.toml"]

[[nightly.lanes]]
id = "nightly"
title = "nightly"
entrypoint = "printf nightly"
command = ["python3", "-c", "print('nightly')"]
""".strip()
                + "\n",
                encoding="utf-8",
            )
            git(repo, "add", "ci/forge-lanes.toml")
            git(repo, "commit", "-m", "manifest change")
            head = git(repo, "rev-parse", "HEAD")

            completed = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "select",
                    "--mode",
                    "branch",
                    "--base",
                    base,
                    "--head",
                    head,
                ],
                cwd=repo,
                check=True,
                text=True,
                capture_output=True,
            )
            payload = json.loads(completed.stdout)
            ids = [lane["id"] for lane in payload["include"]]
            self.assertIn("manifest_only", ids)
            self.assertIn("docs", ids)

    def test_nightly_mode_selects_all_nightly_lanes(self) -> None:
        completed = subprocess.run(
            ["python3", str(SCRIPT), "select", "--mode", "nightly"],
            cwd=REPO_ROOT,
            check=True,
            text=True,
            capture_output=True,
        )
        payload = json.loads(completed.stdout)
        ids = [lane["id"] for lane in payload["include"]]
        self.assertIn("nightly_linux", ids)
        self.assertIn("nightly_apple_host_bundle", ids)

    def test_repo_root_override_reads_manifest_from_external_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            git(repo, "init")
            git(repo, "config", "user.name", "Test User")
            git(repo, "config", "user.email", "test@example.com")
            (repo / "ci").mkdir()
            (repo / "ci" / "forge-lanes.toml").write_text(
                """
version = 1
nightly_schedule_utc = "08:00"

[[branch.lanes]]
id = "override_lane"
title = "override-lane"
entrypoint = "printf override"
command = ["python3", "-c", "print('override')"]
paths = ["README.md"]

[[nightly.lanes]]
id = "override_nightly"
title = "override-nightly"
entrypoint = "printf nightly"
command = ["python3", "-c", "print('nightly')"]
""".strip()
                + "\n",
                encoding="utf-8",
            )
            (repo / "README.md").write_text("override\n", encoding="utf-8")
            git(repo, "add", "README.md", "ci/forge-lanes.toml")
            git(repo, "commit", "-m", "override")

            completed = subprocess.run(
                ["python3", str(SCRIPT), "select", "--mode", "branch", "--all"],
                cwd=REPO_ROOT,
                env={**os.environ, "FORGE_GITHUB_CI_REPO_ROOT": str(repo)},
                check=True,
                text=True,
                capture_output=True,
            )
            payload = json.loads(completed.stdout)
            ids = [lane["id"] for lane in payload["include"]]
            self.assertEqual(ids, ["override_lane"])

    def test_workflow_uses_pull_request_not_pull_request_target(self) -> None:
        workflow = (REPO_ROOT / ".github" / "workflows" / "pre-merge.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("pull_request:\n", workflow)
        self.assertNotIn("pull_request_target:", workflow)
        self.assertIn("path: pr", workflow)
        self.assertIn("FORGE_GITHUB_CI_REPO_ROOT", workflow)


if __name__ == "__main__":
    unittest.main()
