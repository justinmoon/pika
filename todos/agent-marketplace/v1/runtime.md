# Simple Runtime Model (Experiment)

Status: implementation-targeted and intentionally narrow.

## 1. Supported Runtime Backends

- Fly
- MicroVM

No other backend surface in this track.

## 2. Minimal Runtime Record

Required runtime record fields:
- `runtime_id`
- `owner_pubkey`
- `provider` (`fly|microvm`)
- `phase` (`queued|provisioning|ready|teardown|failed`)
- `created_at`
- `expires_at`
- provider handle fields (machine/volume or vm/spawner)

## 3. Teardown Behavior (Must-Have)

- MicroVM: real delete (already implemented)
- Fly: implement real machine+volume cleanup (not manual hint)
- Teardown must be idempotent

## 4. Reaper (Minimal)

One simple loop:
- interval: 30-60s
- scan for expired runtimes (`expires_at <= now`)
- call teardown
- if teardown fails, store retry metadata and try again later

Keep this first version simple:
- no complex policy matrix
- no full resource reconciliation loop yet

## 5. Deferred in This Track

- full capability/distribution model
- untrusted source build support
- separate build service
- advanced workload protocol
