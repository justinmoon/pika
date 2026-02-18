#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/ci/select-lanes.sh [options] [path ...]

Options:
  --from <git-ref>     Diff start ref (requires --to)
  --to <git-ref>       Diff end ref (requires --from)
  --paths-file <file>  Newline-delimited changed paths file
  --all                Force all lanes true
  --help               Show this help

Output (shell assignments):
  selected_lanes=<csv>
  run_pika=<true|false>
  run_marmotd=<true|false>
  run_rmp=<true|false>
  run_rapture=<true|false>
  run_notifications=<true|false>
  any_lane=<true|false>
USAGE
}

from_ref=""
to_ref=""
paths_file=""
force_all=false

declare -a input_paths=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from)
      from_ref="${2:-}"
      shift 2
      ;;
    --to)
      to_ref="${2:-}"
      shift 2
      ;;
    --paths-file)
      paths_file="${2:-}"
      shift 2
      ;;
    --all)
      force_all=true
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      input_paths+=("$1")
      shift
      ;;
  esac
done

if [[ -n "$from_ref" || -n "$to_ref" ]]; then
  if [[ -z "$from_ref" || -z "$to_ref" ]]; then
    echo "error: --from and --to must be provided together" >&2
    exit 2
  fi
fi

if [[ -n "$paths_file" && ! -f "$paths_file" ]]; then
  echo "error: paths file not found: $paths_file" >&2
  exit 2
fi

run_pika=false
run_marmotd=false
run_rmp=false
run_rapture=false
run_notifications=false

set_all_lanes() {
  run_pika=true
  run_marmotd=true
  run_rmp=true
  run_rapture=true
  run_notifications=true
}

mark_for_path() {
  local p="$1"

  case "$p" in
    "" )
      return
      ;;

    # Pure docs/planning files can skip heavy CI lanes.
    docs/*|prompts/*|todos/*|README.md|AGENTS.md|CLAUDE.md)
      return
      ;;

    # Rapture app lane.
    apps/rapture/*)
      run_rapture=true
      return
      ;;

    # RMP tooling lane.
    crates/rmp-cli/*|docs/rmp-ci.md|todos/rmp-*.md)
      run_rmp=true
      return
      ;;

    # marmotd lane.
    crates/marmotd/*|openclaw-marmot/*)
      run_marmotd=true
      return
      ;;

    # Notifications lane.
    crates/pika-notifications/*)
      run_notifications=true
      return
      ;;

    # Shared media crate affects both apps.
    crates/pika-media/*)
      run_pika=true
      run_rapture=true
      return
      ;;

    # Pika app lane.
    apps/pika/ios/*|apps/pika/android/*|apps/pika/rust/*|apps/pika/cli/*|apps/pika/uniffi-bindgen/*|apps/pika/rmp.toml|tools/pika-run*|crates/pika-nse/*|crates/pika-tls/*)
      run_pika=true
      return
      ;;

    # Global shared config/tooling: safest fan-out.
    .github/workflows/*|justfile|flake.nix|flake.lock|Cargo.toml|Cargo.lock|scripts/*|nix/*)
      set_all_lanes
      return
      ;;

    # Unknown paths default to full fan-out for safety.
    *)
      set_all_lanes
      return
      ;;
  esac
}

if [[ "$force_all" == true ]]; then
  set_all_lanes
else
  if [[ -n "$paths_file" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
      input_paths+=("$line")
    done < "$paths_file"
  fi

  if [[ -n "$from_ref" && -n "$to_ref" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
      input_paths+=("$line")
    done < <(git diff --name-only "$from_ref" "$to_ref")
  fi

  for p in "${input_paths[@]}"; do
    mark_for_path "$p"
  done
fi

lanes=()
[[ "$run_pika" == true ]] && lanes+=("pika")
[[ "$run_marmotd" == true ]] && lanes+=("marmotd")
[[ "$run_rmp" == true ]] && lanes+=("rmp")
[[ "$run_rapture" == true ]] && lanes+=("rapture")
[[ "$run_notifications" == true ]] && lanes+=("notifications")

selected_lanes=""
if [[ ${#lanes[@]} -gt 0 ]]; then
  selected_lanes="$(IFS=,; echo "${lanes[*]}")"
fi

any_lane=false
[[ ${#lanes[@]} -gt 0 ]] && any_lane=true

echo "selected_lanes=$selected_lanes"
echo "run_pika=$run_pika"
echo "run_marmotd=$run_marmotd"
echo "run_rmp=$run_rmp"
echo "run_rapture=$run_rapture"
echo "run_notifications=$run_notifications"
echo "any_lane=$any_lane"
