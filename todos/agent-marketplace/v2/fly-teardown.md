# Fly Teardown Spec (MVP)

Status: deferred reference mini-spec; active teardown implementation lives under v1.

## 1. Goal

Replace manual/advisory Fly teardown with real cleanup and deterministic outcomes.

## 2. Scope

Applies to runtime records backed by Fly machine + Fly volume handles.

Inputs:
- `machine_id`
- `volume_id`
- `app_name`

## 3. API Calls (in order)

Given Fly base URL:
- `BASE = {api_base}/v1/apps/{app_name}`

Sequence:
1. Best-effort stop machine
- `POST {BASE}/machines/{machine_id}/stop`
- Body: `{ "signal": "SIGTERM", "timeout": "10s" }` (or provider-default if omitted)
- If 404: treat as already stopped/gone.

2. Delete machine
- `DELETE {BASE}/machines/{machine_id}?force=true`
- If 404: treat as already gone.

3. Delete volume
- `DELETE {BASE}/volumes/{volume_id}`
- If 404: treat as already gone.
- If attach/in-use conflict (409/422 style provider conflict): classify retryable.

## 4. Idempotency and Outcome Contract

Teardown must be idempotent.

Structured outcomes:
- `deleted`: machine+volume deleted now.
- `already_gone`: resources absent before/at deletion attempt.
- `partial`: machine deleted but volume conflict (retryable).
- `failed`: teardown failed (retryable or terminal must be explicit).

Required payload fields:
- `teardown`
- `machine_id`
- `volume_id`
- `app_name`
- `machine_status` (`deleted|already_gone|failed`)
- `volume_status` (`deleted|already_gone|conflict|failed`)
- `retryable` (bool)
- `error` (optional structured detail)

## 5. Server Behavior

1. Persist runtime phase transition to teardown before external calls.
2. Execute the ordered API sequence.
3. Persist final status with retry metadata if not fully deleted.
4. Reaper retries only retryable failures.

## 6. Retry Policy (initial)

- exponential backoff with jitter
- cap max interval
- persist `attempt_count`, `last_error`, `next_retry_at`

## 7. Tests

Required tests:
1. machine 404 + volume 404 => `already_gone`, success.
2. machine deleted + volume deleted => `deleted`, success.
3. machine deleted + volume conflict => `partial`, retryable.
4. transport/provider error => `failed` with retry classification.
5. repeated teardown calls remain idempotent.
