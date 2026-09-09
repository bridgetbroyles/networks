# Assignment 1 test plan and final results

The purpose of testing is not only to make the report card green. It must show discovery completes, final routes are correct, failures and recoveries converge, coordination cannot install stale state, and safety does not cause a permanent black hole.

## Success criteria

- Part 1: C1 at least 99.9%, complete C2 reachability, zero steady-state loss, ideally zero C3 loops.
- Part 2: C1 at least 98%, complete C2, zero persistent or transient loops, and all C4 events at most 1,000 ms.
- No panic, WASM trap, fuel exhaustion, invalid output, action-budget overflow, frozen controller, or table-capacity failure.
- Every withdrawn route is reinstalled when reachable again.
- Old, duplicate, or out-of-order control information cannot replace newer decisions.

## Build and unit tests

Final verification on 2026-09-09:

- A1 crate: 21 tests passed, 0 failed.
- Repository/SDK/simulator workspace: 93 tests passed, 0 failed; two ignored doctests.
- Release WASM build: passed for `wasm32-unknown-unknown`.
- Formatting check: passed.

The A1 tests cover:

- HELLO, LSA, READY, UPDATE_PHASE, and PHASE_ACK round trips;
- malformed/truncated/oversized payload rejection;
- strict HELLO/LSA freshness;
- delayed customer classification;
- timeout and fresh-HELLO recovery;
- mutual graph, sparse component, control next hop, deterministic tie break, and decreasing BFS distance;
- unchanged-route suppression and delete-before-install replacement;
- distance-specific phase application;
- all-member READY and ACK barriers;
- phase retry;
- cancellation by a newer LSA;
- rejection of an older generation after a newer one is active.

## Part 1 practice worlds

| World | Shape / scale | Delivery | Reachability | Loops / TTL | Result |
|---|---|---:|---:|---:|---|
| `practice-ring-002` | ring, 6 switches | 100.00%, 0/5,814 lost | 30/30 | 0 / 0 | PASS |
| `practice-hub-009` | hub, 7 switches | 100.00%, 0/6,783 lost | 42/42 | 0 / 0 | PASS |
| `practice-grid-003` | grid, 10 switches | 100.00%, 0/9,690 lost | 90/90 | 0 / 0 | PASS |
| `practice-dumb-006` | dumbbell, 13 switches | 100.00%, 0/12,597 lost | 156/156 | 0 / 0 | PASS |
| `practice-ring-010` | ring/chords, 15 switches | 100.00%, 0/14,535 lost | 210/210 | 0 / 0 | PASS |

What this establishes:

- startup HELLOs distinguish every inter-switch port before the 50 ms customer decision;
- all customer addresses and LSAs spread before the 200 ms scoring window;
- readiness barriers do not deadlock during startup;
- every ordered pair obtains a route;
- larger diameter and 15-switch state remain within resource limits.

Expected unscored startup behavior: early route misses are punted and lost while customer locations are being learned. They are not evidence of a forwarding failure after warmup.

## Part 2 failure schedules

Every supplied schedule contains six failure intervals and therefore 12 scored DOWN/UP events.

| Schedule | Delivery (lost/total) | Reachability | Loops / TTL | Events | Worst | No route |
|---|---:|---:|---:|---:|---:|---:|
| `part2-dumb-006-f001` | 99.54% (149/32,071) | 156/156 | 0 / 0 | 12/12 | 128 ms | 1 |
| `part2-dumb-006-f002` | 99.61% (125/32,071) | 156/156 | 0 / 0 | 12/12 | 117 ms | 5 |
| `part2-dumb-006-f003` | 99.64% (116/32,435) | 156/156 | 0 / 0 | 12/12 | 105 ms | 5 |
| `part2-grid-003-f001` | 99.57% (116/27,110) | 90/90 | 0 / 0 | 12/12 | 108 ms | 4 |
| `part2-grid-003-f002` | 99.64% (95/26,660) | 90/90 | 0 / 0 | 12/12 | 98 ms | 1 |
| `part2-grid-003-f003` | 99.56% (103/23,390) | 90/90 | 0 / 0 | 12/12 | 123 ms | 4 |
| `part2-hub-009-f001` | 99.61% (63/15,995) | 42/42 | 0 / 0 | 12/12 | 98 ms | 0 |
| `part2-hub-009-f002` | 99.61% (70/18,130) | 42/42 | 0 / 0 | 12/12 | 95 ms | 7 |
| `part2-hub-009-f003` | 99.64% (65/18,025) | 42/42 | 0 / 0 | 12/12 | 90 ms | 0 |
| `part2-ring-002-f001` | 99.58% (65/15,636) | 30/30 | 0 / 0 | 12/12 | 112 ms | 1 |
| `part2-ring-002-f002` | 99.61% (59/14,940) | 30/30 | 0 / 0 | 12/12 | 112 ms | 4 |
| `part2-ring-002-f003` | 99.67% (51/15,366) | 30/30 | 0 / 0 | 12/12 | 116 ms | 2 |
| `part2-ring-010-f001` | 99.60% (148/36,720) | 210/210 | 0 / 0 | 12/12 | 135 ms | 14 |
| `part2-ring-010-f002` | 99.65% (139/39,630) | 210/210 | 0 / 0 | 12/12 | 145 ms | 19 |
| `part2-ring-010-f003` | 99.65% (134/37,875) | 210/210 | 0 / 0 | 12/12 | 121 ms | 7 |

All 15 runs passed every displayed criterion. Across them, 180/180 events recovered, no program failed, and no route stayed absent. The maximum recovery is only 14.5% of the allowed budget.

## Focused behavior checks

### Normal forwarding and route invariant

For any destination, reconstruct the owner and BFS distances from the logged table state. Every installed nonlocal next hop should be adjacent and have a distance one smaller. The destination owner should forward to its customer port. This proves settled routes cannot loop.

### Startup and warmup

Inspect 0, 25, 50, 100, and 200 ms:

1. HELLOs identify switch ports.
2. Ordinary punts create customer candidates.
3. At 50 ms, candidates become local customers and LSAs change.
4. Identical views produce READY messages.
5. Distance phases install routes outward.
6. At 200 ms, all scored pairs forward without punts.

Do not increase warmup to hide incomplete discovery.

### Topology and customer discovery

- Verify neighbor IDs are learned from HELLO ingress, not world filenames or port numbering.
- Verify local LSAs change only when local facts change.
- Verify only traffic from unknown/customer ports is learned locally; a no-route transit packet on a switch port must not become a false customer.
- Verify exact entries use observed addresses because prefix length is absent from the API.

### Link failure

For a DOWN event, inspect:

- last fresh HELLO and 100 ms timeout;
- immediate deletion of routes using the dead local port;
- newer endpoint LSA and removal of mutual adjacency;
- cancellation of any older update;
- common-view READY barrier;
- phase 0 withdrawals, then increasing-distance installs;
- report-card time of the last lost packet.

### Link recovery

Confirm HELLOs continued while down, a fresh sequence arrived after restore, the endpoint replied/synchronized, both endpoint LSAs restored mutual adjacency, and ordered phases installed the recovered shorter paths. UP events must pass C4 just like DOWN events.

### Stale, duplicate, and out-of-order information

Checks must establish:

- equal/older HELLOs do not refresh liveness;
- equal/older LSAs do not replace or reflood an origin;
- READY must match current component leader and exact view;
- an older generation cannot replace a newer active update;
- out-of-order phases are rejected;
- a duplicate completed phase can be re-ACKed but does not repeat route actions;
- delayed ACKs cannot advance a different generation/view/phase;
- a new LSA supersedes in-progress coordination immediately.

### Loops and black holes

C3 and packet-hop analysis must both report zero repeated switches and zero TTL deaths. A no-route punt is not automatically a bug: phase 0 intentionally withdraws unsafe paths. It is a bug if reachability does not return, C2 is incomplete, loss continues beyond C4, or a route is never reinstalled.

Final official totals were 74 temporary no-route punts among 1,498 losses. All reachability and C4 checks passed. The two checkpoint loop regressions changed from six loops each to zero.

### Coordination liveness and action limits

- Look for a leader stuck on one phase, repeated attempts without ACK completion, or `routing_dirty` that never clears.
- Verify a retry reaches branches missed by an earlier flooded attempt.
- On the 15-switch world, verify no 4,096-byte output, instruction, or table-capacity failure.
- If a route batch does not fit, confirm no ACK is sent; retry should apply only remaining diffs and then ACK.

No such failure appeared in the official matrix.

## Rapid-change stress test

A temporary harness, outside the repository, used the existing `part2-grid-003` world and unmodified simulator. It changed links 5--85 ms apart so a new view could supersede a barrier or phase already in flight.

Result: 98.22% delivery, 90/90 reachability, zero loops/TTL, 6/6 events within budget, 50 ms worst recovery, zero no-route losses, and no program failure. This covers failure during coordination, recovery during coordination, rapid successive views, and multiple path lengths.

The official failure parser intentionally requires nonoverlapping changes at least 1,000 ms apart, so this test calls the existing simulator's public fail/restore operations directly rather than altering schedules or validation rules.

## Performance comparison

- Checkpoint losses: 1,935; ordered implementation: 1,498 (-22.6%).
- Recovery: improved on 14 schedules and tied on one; new worst is 145 ms.
- Loops: 12 total checkpoint observations across two schedules; zero now.
- Control ingress hops: +24.4%; control hop-bytes: +35.8%.
- Release WASM size: +15.2%.

This means the safety improvement did not buy zero loops by causing excessive loss or slower convergence. The additional control work is modest at A1 scale.

## Report-card interpretation

- **C1 failure:** separate failed-link/queue loss from no-route loss and look for prolonged silence.
- **C2 failure:** identify the missing ordered pair and trace the destination's customer LSA and distance phases.
- **C3 failure:** inspect mixed route generations and repeated hop sequence.
- **C4 failure:** inspect the exact DOWN/UP row; recovery is the last lost packet's send time, not merely the first later success.
- **Program failed:** resolve the trap/fuel/output issue before interpreting routing symptoms; the data plane otherwise freezes at its old state.

## Logs and replay viewer

Keep one uniquely named `.simlog` per run. The automatic scratch name is world-based and collides if schedules for the same world run concurrently.

Use the scorer and viewer to inspect:

- report-card example packets;
- failed/recovered links;
- table delete/install order and timestamps;
- whether a route is absent briefly or permanently;
- repeated switch hops and TTL deaths;
- reuse of restored links.

Control payload bytes are not included in the generic log, so exact protocol-state assertions belong in unit tests or a temporary external analyzer, not simulator modifications.

## Reproducible command outline

```sh
cargo test --manifest-path a1_switch/Cargo.toml
cargo build --release --target wasm32-unknown-unknown \
  --manifest-path a1_switch/Cargo.toml
cargo test --workspace

for world in worlds/practice/practice-*.toml; do
  competitive_net_sim run-world "$world" --program "$PROGRAM" --score
done

for schedule in worlds/practice/failures/*.toml; do
  # Derive its matching part2 world and give every run a unique --log path.
  competitive_net_sim run-world "$WORLD" --failures "$schedule" \
    --program "$PROGRAM" --score --log "$UNIQUE_LOG"
done
```

## Hidden-world risks to retain in final review

- shuffled/sparse switch IDs and ties;
- largest diameter/component and high-degree nodes;
- topology change just before/after HELLO timeout or phase retry;
- dropped control flood/ACK;
- component leader removed by a failure;
- repeated supersession that eventually stops;
- multiple simultaneous failures if the graph remains connected;
- any assignment interpretation requiring unseen addresses inside a prefix.

The code derives behavior only from observable ports, messages, packet addresses, and time. It does not key on practice topology names, switch numbering patterns, failure times, or the known app port.
