#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath


def repo_root() -> Path:
    override = os.environ.get("FORGE_GITHUB_CI_REPO_ROOT")
    if override:
        return Path(override).resolve()
    cwd = Path.cwd().resolve()
    if (cwd / "ci" / "forge-lanes.toml").exists():
        return cwd
    return Path(__file__).resolve().parent.parent


def manifest_path() -> Path:
    return repo_root() / "ci" / "forge-lanes.toml"


def load_manifest(path: Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def match_path(path: str, patterns: list[str]) -> bool:
    posix = PurePosixPath(path)
    for pattern in patterns:
        if posix.match(pattern):
            return True
    return False


def changed_paths(base: str, head: str) -> list[str]:
    output = subprocess.check_output(
        ["git", "diff", "--name-only", f"{base}..{head}"],
        cwd=repo_root(),
        text=True,
    )
    return [line.strip() for line in output.splitlines() if line.strip()]


def lane_catalog(manifest: dict, mode: str) -> list[dict]:
    group = manifest.get(mode, {})
    return list(group.get("lanes", []))


def lane_timeout_minutes(lane_id: str) -> int:
    if lane_id == "nightly_apple_host_bundle":
        return 90
    if lane_id == "nightly_pika_ui_android":
        return 75
    return 60


def lane_to_matrix_entry(lane: dict, mode: str) -> dict:
    command = list(lane["command"])
    uses_apple_remote = any("pikaci-apple-remote.sh" in part for part in command)
    return {
        "id": lane["id"],
        "title": lane["title"],
        "entrypoint": lane["entrypoint"],
        "command": command,
        "command_shell": shlex.join(command),
        "mode": mode,
        "runner": "ubuntu-latest",
        "timeout_minutes": lane_timeout_minutes(lane["id"]),
        "needs_openclaw_checkout": mode == "nightly" and lane["id"] == "nightly_pikachat",
        "needs_gradle_cache": mode == "nightly" and lane["id"] == "nightly_pika_ui_android",
        "uses_apple_remote": uses_apple_remote,
    }


def select_lanes(manifest: dict, mode: str, base: str | None, head: str | None, force_all: bool) -> dict:
    lanes = lane_catalog(manifest, mode)
    changed = []
    selected = []
    if mode == "nightly" or force_all or not base or not head:
        selected = lanes
    else:
        changed = changed_paths(base, head)
        if not changed or "ci/forge-lanes.toml" in changed:
            selected = lanes
        else:
            selected = [
                lane
                for lane in lanes
                if not lane.get("paths") or match_path_any(changed, lane.get("paths", []))
            ]
    return {
        "mode": mode,
        "manifest_path": "ci/forge-lanes.toml",
        "changed_paths": changed,
        "selected_count": len(selected),
        "selected_titles": [lane["title"] for lane in selected],
        "include": [lane_to_matrix_entry(lane, mode) for lane in selected],
    }


def match_path_any(paths: list[str], patterns: list[str]) -> bool:
    return any(match_path(path, patterns) for path in paths)


def write_github_output(path: str, payload: dict) -> None:
    with open(path, "a", encoding="utf-8") as fh:
        fh.write("matrix<<__FORGE_MATRIX__\n")
        fh.write(json.dumps(payload["include"], separators=(",", ":")))
        fh.write("\n__FORGE_MATRIX__\n")
        fh.write(f"selected_count={payload['selected_count']}\n")
        fh.write("selected_titles<<__FORGE_TITLES__\n")
        fh.write("\n".join(payload["selected_titles"]))
        fh.write("\n__FORGE_TITLES__\n")
        fh.write("changed_paths<<__FORGE_CHANGED__\n")
        fh.write("\n".join(payload["changed_paths"]))
        fh.write("\n__FORGE_CHANGED__\n")


def cmd_select(args: argparse.Namespace) -> int:
    manifest = load_manifest(manifest_path())
    payload = select_lanes(manifest, args.mode, args.base, args.head, args.all)
    if args.github_output:
        write_github_output(args.github_output, payload)
    print(json.dumps(payload, indent=2))
    return 0


def find_lane(manifest: dict, mode: str, lane_id: str) -> dict:
    for lane in lane_catalog(manifest, mode):
        if lane["id"] == lane_id:
            return lane
    raise SystemExit(f"unknown {mode} lane: {lane_id}")


def cmd_run(args: argparse.Namespace) -> int:
    manifest = load_manifest(manifest_path())
    lane = find_lane(manifest, args.mode, args.lane_id)
    command = list(lane["command"])
    print(f"running {lane['title']}: {shlex.join(command)}", flush=True)
    completed = subprocess.run(command, cwd=repo_root())
    return completed.returncode


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    select = sub.add_parser("select")
    select.add_argument("--mode", choices=["branch", "nightly"], required=True)
    select.add_argument("--base")
    select.add_argument("--head")
    select.add_argument("--all", action="store_true")
    select.add_argument("--github-output")
    select.set_defaults(func=cmd_select)

    run = sub.add_parser("run")
    run.add_argument("--mode", choices=["branch", "nightly"], required=True)
    run.add_argument("--lane-id", required=True)
    run.set_defaults(func=cmd_run)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
