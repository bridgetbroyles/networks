# Assignment 1 implementation log

This is the evidence record for the current implementation. Proposed architecture is explained in `A1_DESIGN.md`.

## Final status on 2026-09-09

- Git baseline: `636480b` (`dry run`) on branch `implementation1`, also at `origin/implementation1`.
- Current assignment changes remain uncommitted by request.
- Code changed from the baseline only in `a1_switch/src/lib.rs`, `protocol.rs`, and `routing.rs`.
- All five A1 documents were updated after the code and validation were settled.
- No simulator, scorer, SDK, world, or failure-schedule source was changed.
- Recommendation after validation: keep the current implementation.

## Original implementation at the checkpoint

The checkpoint already had a working link-state program:

- two exact TinyVM tables (reserved control protocol, then customer destination);
- 25 ms HELLOs, 50 ms customer classification, and 100 ms dead-neighbor timeout;
- complete sequence-numbered LSAs with flooding and rotating anti-entropy;
- mutual-adjacency BFS and deterministic next-hop selection;
- exact observed customer `/32` routes with safe delete-before-install diffs;
- immediate withdrawal of routes using a newly dead local port;
- continued HELLO probing and LSDB synchronization on recovery.

It passed all required A1 criteria, but `part2-grid-003-f002` and `part2-dumb-006-f003` each showed six transient packet loops because switches installed a new SPF independently.

## Ordered-update implementation

### Files changed

- `a1_switch/src/protocol.rs`
  - Added bounded READY, UPDATE_PHASE, and PHASE_ACK messages.
  - Added `ViewId`, generation, phase, attempt, and sender/leader fields.
  - Added round-trip and malformed-message tests.
- `a1_switch/src/routing.rs`
  - Added BFS `distance` to `DesiredRoute`.
  - Added mutual-component membership and deterministic control-path next-hop helpers.
  - Added graph/path/phase-distance tests.
- `a1_switch/src/lib.rs`
  - Added exact LSDB view hashing, READY barrier, lowest-ID component leader, generation tracking, distance phases, ACK collection, retry attempts, and supersession.
  - Route phase 0 handles withdrawals/local routes; phases 1 through component size minus one update increasing destination distance.
  - A route phase is acknowledged only after every matching route action fits and is submitted.
  - New LSAs cancel obsolete coordination. Old generations and mismatched view/leader/phase messages are ignored.
  - Removed the former blind 5 ms SPF stability hold. Exact-view agreement now supplies the needed synchronization without adding a full timer tick after the final LSA.

### Why the first ordered iteration was adjusted

The first version waited 5 ms after every view change before sending READY. It eliminated loops, but targeted recovery became slower: grid f002 reached 170 losses/169 ms and dumb f003 reached 210 losses/173 ms. The barrier already requires every member to report the exact final view, so the extra fixed hold added latency without adding safety. Sending READY immediately retained zero loops and improved the same runs to 95 losses/98 ms and 116 losses/105 ms respectively.

## Unit and build verification

Temporary Rust 1.98.1 tooling and the WASM target were kept under `/private/tmp`; global toolchain configuration was not changed.

- `cargo test --manifest-path a1_switch/Cargo.toml`: 21 passed, 0 failed.
- Release `wasm32-unknown-unknown` build: passed.
- `cargo fmt --manifest-path a1_switch/Cargo.toml -- --check`: passed.
- Release module size: 204,194 bytes; checkpoint module: 177,327 bytes (+26,867, +15.2%).
- `cargo test --workspace --quiet`: 93 passed, 0 failed; two doctests are marked ignored.
- One existing SDK unit-test warning reports an unread `ToyProgram.switch_id`; it is unrelated to A1.

## Part 1 results

| World | Delivery | Reachability | Loops / TTL | Result |
|---|---:|---:|---:|---|
| `practice-ring-002` | 100.00%, 0/5,814 lost | 30/30 | 0 / 0 | PASS |
| `practice-hub-009` | 100.00%, 0/6,783 lost | 42/42 | 0 / 0 | PASS |
| `practice-grid-003` | 100.00%, 0/9,690 lost | 90/90 | 0 / 0 | PASS |
| `practice-dumb-006` | 100.00%, 0/12,597 lost | 156/156 | 0 / 0 | PASS |
| `practice-ring-010` | 100.00%, 0/14,535 lost | 210/210 | 0 / 0 | PASS |

There were no crashes, traps, assertions, capacity failures, or resource warnings.

## Part 2 final results

Every row passed C1, C2, C3, and C4. Every schedule has 12 scored link events.

| Schedule | Delivery (lost/total) | Reachability | Loops / TTL | C4 worst | No-route drops |
|---|---:|---:|---:|---:|---:|
| `dumb-006-f001` | 99.54% (149/32,071) | 156/156 | 0 / 0 | 128 ms | 1 |
| `dumb-006-f002` | 99.61% (125/32,071) | 156/156 | 0 / 0 | 117 ms | 5 |
| `dumb-006-f003` | 99.64% (116/32,435) | 156/156 | 0 / 0 | 105 ms | 5 |
| `grid-003-f001` | 99.57% (116/27,110) | 90/90 | 0 / 0 | 108 ms | 4 |
| `grid-003-f002` | 99.64% (95/26,660) | 90/90 | 0 / 0 | 98 ms | 1 |
| `grid-003-f003` | 99.56% (103/23,390) | 90/90 | 0 / 0 | 123 ms | 4 |
| `hub-009-f001` | 99.61% (63/15,995) | 42/42 | 0 / 0 | 98 ms | 0 |
| `hub-009-f002` | 99.61% (70/18,130) | 42/42 | 0 / 0 | 95 ms | 7 |
| `hub-009-f003` | 99.64% (65/18,025) | 42/42 | 0 / 0 | 90 ms | 0 |
| `ring-002-f001` | 99.58% (65/15,636) | 30/30 | 0 / 0 | 112 ms | 1 |
| `ring-002-f002` | 99.61% (59/14,940) | 30/30 | 0 / 0 | 112 ms | 4 |
| `ring-002-f003` | 99.67% (51/15,366) | 30/30 | 0 / 0 | 116 ms | 2 |
| `ring-010-f001` | 99.60% (148/36,720) | 210/210 | 0 / 0 | 135 ms | 14 |
| `ring-010-f002` | 99.65% (139/39,630) | 210/210 | 0 / 0 | 145 ms | 19 |
| `ring-010-f003` | 99.65% (134/37,875) | 210/210 | 0 / 0 | 121 ms | 7 |

Totals: 1,498 scored losses; 74 were no-route punts and 1,424 were failed-link/queue drops. No route remained absent, all ordered pairs delivered, all 180 events recovered, and there were no program failures. Worst recovery was 145 ms, leaving 855 ms of margin.

## Baseline versus improved protocol

| Schedule | Baseline loss / worst | Improved loss / worst | Loop change |
|---|---:|---:|---:|
| `dumb-f001` | 200 / 148 ms | 149 / 128 ms | 0 -> 0 |
| `dumb-f002` | 155 / 137 ms | 125 / 117 ms | 0 -> 0 |
| `dumb-f003` | 156 / 145 ms | 116 / 105 ms | 6 -> 0 |
| `grid-f001` | 155 / 141 ms | 116 / 108 ms | 0 -> 0 |
| `grid-f002` | 129 / 136 ms | 95 / 98 ms | 6 -> 0 |
| `grid-f003` | 130 / 143 ms | 103 / 123 ms | 0 -> 0 |
| `hub-f001` | 94 / 140 ms | 63 / 98 ms | 0 -> 0 |
| `hub-f002` | 95 / 135 ms | 70 / 95 ms | 0 -> 0 |
| `hub-f003` | 96 / 133 ms | 65 / 90 ms | 0 -> 0 |
| `ring002-f001` | 87 / 140 ms | 65 / 112 ms | 0 -> 0 |
| `ring002-f002` | 78 / 132 ms | 59 / 112 ms | 0 -> 0 |
| `ring002-f003` | 68 / 144 ms | 51 / 116 ms | 0 -> 0 |
| `ring010-f001` | 166 / 140 ms | 148 / 135 ms | 0 -> 0 |
| `ring010-f002` | 171 / 145 ms | 139 / 145 ms | 0 -> 0 |
| `ring010-f003` | 155 / 142 ms | 134 / 121 ms | 0 -> 0 |

Loss fell on all 15 schedules: 1,935 to 1,498 (-437, -22.6%). Worst recovery improved on 14 schedules and tied on one. The two observed loop cases became zero.

## Control and action overhead

The simulator assigns the same packet ID to injected control packets, so unique-packet counting is not meaningful. Log analysis instead counted control packet ingress hops and the bytes represented by those hops across all schedules.

| Metric | Checkpoint | Ordered update | Increase |
|---|---:|---:|---:|
| Control ingress hops | 792,295 | 985,890 | 193,595 (+24.4%) |
| Control hop-bytes | 37,356,302 | 50,722,440 | 13,366,138 (+35.8%) |

The aggregate ordered-update rate is about 1.08 Mbps across the network, negligible beside 1 Gbps links. READY/ACK traffic is unicast; phases are flooded. No callback exceeded the action output limit. If a route batch ever did not fit, it would not ACK, and the next phase attempt would resume with already-applied actions omitted.

## Rapid-change stress replay

A temporary external harness used the unmodified `part2-grid-003` world and simulator APIs. It applied six down/up events 5--85 ms apart, including failure and recovery while older coordination could still be active.

- 98.22% delivery (39/2,190 lost);
- 90/90 reachability;
- zero loops and zero TTL drops;
- all six events within 1,000 ms, worst 50 ms;
- all 39 losses were failed-link/queue drops, zero no-route drops;
- no crash, trap, assertion, or permanent wait.

This specifically exercises view supersession, different distance phases, and recovery during coordination. The harness lives outside the assignment checkout and is not a submission change.

## Failures encountered and fixes

1. **Transient loops in the checkpoint.** Cause: independent FIB installation from mixed LSDB views. Fix: exact-view barrier plus destination-rooted phases and ACKs. Result: zero loops in both regressions and the full matrix.
2. **First ordered version added latency.** Cause: redundant 5 ms readiness hold, observed only on the next 25 ms timer. Fix: announce each new view immediately; exact-view agreement preserves safety. Result: both targeted loss/recovery metrics improved beyond baseline.
3. **Earlier parallel score logs collided.** Cause: automatic score-log names are world-based. Fix: unique log file per schedule. This was a test harness issue, not a protocol defect.
4. **Redirected build target broke one integration path.** Cause: the test expects its example WASM under the conventional repository target path. A conventional/cached rerun passed; no source change was needed.

## Remaining work

No implementation change is justified by current evidence. Remaining submission work is review, instructor-required packaging if any, and a user-authorized commit. The main residual risks are theoretical view-hash collision, endless adversarial topology churn, and exact-host rather than unknowable prefix-wide routing; these are documented in `A1_DESIGN.md`.
