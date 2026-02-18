#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
selector="$repo_root/scripts/ci/select-lanes.sh"
fixtures_dir="$repo_root/scripts/ci/tests/fixtures"

if [[ ! -x "$selector" ]]; then
  echo "selector script missing or not executable: $selector" >&2
  exit 1
fi

failures=0

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  if [[ "$got" != "$want" ]]; then
    echo "FAIL: $label (got='$got' want='$want')" >&2
    failures=$((failures + 1))
  fi
}

expect_flag() {
  local lane="$1"
  local expected_csv="$2"
  local expected="false"
  if [[ ",$expected_csv," == *",$lane,"* ]]; then
    expected="true"
  fi

  local var="run_${lane}"
  local got="${!var}"
  assert_eq "$got" "$expected" "$case_name:$var"
}

for paths_file in "$fixtures_dir"/*.paths; do
  case_name="$(basename "$paths_file" .paths)"
  expected_file="$fixtures_dir/$case_name.expected"
  if [[ ! -f "$expected_file" ]]; then
    echo "missing expected file: $expected_file" >&2
    exit 1
  fi

  expected_csv="$(tr -d '\r' < "$expected_file" | head -n 1)"

  # shellcheck disable=SC1090
  eval "$($selector --paths-file "$paths_file")"

  assert_eq "$selected_lanes" "$expected_csv" "$case_name:selected_lanes"

  expect_flag pika "$expected_csv"
  expect_flag marmotd "$expected_csv"
  expect_flag rmp "$expected_csv"
  expect_flag rapture "$expected_csv"
  expect_flag notifications "$expected_csv"

  expected_any="false"
  [[ -n "$expected_csv" ]] && expected_any="true"
  assert_eq "$any_lane" "$expected_any" "$case_name:any_lane"
done

if [[ "$failures" -gt 0 ]]; then
  echo "lane selection tests failed: $failures" >&2
  exit 1
fi

echo "lane selection tests passed"
