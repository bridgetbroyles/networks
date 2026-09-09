# Assignment 1 implementation plan and completed progress

This is the implementation map for the code now in `a1_switch/`. It preserves the original Part 1-first development order and records how Part 2 hardening was added without changing the simulator or grader.

## Current completion

- Part 1 discovery, exact forwarding, and settled shortest-path routing: complete; 5/5 worlds pass with 100% scored delivery.
- Part 2 silent failure/recovery: complete; 15/15 schedules pass all required criteria.
- Transient-loop prevention: complete; all official Part 2 runs now report zero loops and zero TTL drops.
- Unit/repository verification: 21 A1 tests and 93 repository tests pass.
- Documentation: updated to the finalized ordered-update implementation.
- Commit: intentionally not created; the user will decide when to commit.

## Files

### Assignment implementation

- `a1_switch/Cargo.toml`: standalone Rust 2024 `cdylib`/`rlib` crate using the supplied SDK.
- `a1_switch/Cargo.lock`: reproducible dependency resolution.
- `a1_switch/src/lib.rs`: switch state, TinyVM setup, discovery, timers, LSDB handling, coordination, FIB updates, and action budgeting.
- `a1_switch/src/protocol.rs`: bounded codecs for HELLO, LSA, READY, UPDATE_PHASE, and PHASE_ACK.
- `a1_switch/src/routing.rs`: mutual graph, component membership, BFS, control next hops, and desired customer routes with distance.

### Documentation

- `docs/A1_DESIGN.md`: architecture and reasoning.
- `docs/A1_IMPLEMENTATION_PLAN.md`: this code-oriented map.
- `docs/A1_IMPLEMENTATION_LOG.md`: evidence, failures, fixes, and measurements.
- `docs/A1_TEST_PLAN.md`: validation strategy and complete results.
- `docs/A1_QUIZ_GUIDE.md`: student-oriented explanation.

No file in simulator `src/`, grader/scorer, SDK, practice worlds, or failure schedules is changed.

## Major structs and state

### `A1Switch`

Core fields are:

- identity and `ports: BTreeMap<u16, PortState>`;
- local HELLO/LSA counters and `lsdb: BTreeMap<u32, Lsa>`;
- `local_customers` and `installed_routes`;
- routing/anti-entropy state;
- readiness and view state;
- `active_update` for the generation being applied;
- `leader_update` when this switch is the component leader;
- `latest_generation` and `seen_phase_attempt` for stale/duplicate rejection.

### Discovery/routing types

- `PortState`: role, HELLO freshness, classification timestamp, and candidate customers.
- `Lsa`: origin, sequence, sorted neighbors, sorted customer addresses.
- `DesiredRoute`: output port, optional next-hop switch, and BFS distance.
- `InstalledRoute`: stable table-entry ID, output port, optional next hop.

### Coordination types

- `ViewId { high, low }`: deterministic 128-bit LSDB identity.
- `Ready`: sender/leader/readiness sequence/view.
- `ActiveUpdate`: exact view and generation plus frozen desired routes and completed phase.
- `LeaderUpdate`: members, current phase/attempt, ACK set, and retry timestamp.
- `UpdatePhase` and `PhaseAck`: exact leader/generation/view/phase identities.

### `ActionBatch`

Tracks estimated encoded size against a 3,200-byte conservative ceiling, reserves room for the next timer, and makes route-phase completion explicit. An incomplete phase is not acknowledged.

## Important functions

### Setup and dispatch

- `init`: declares control/route tables and two TinyVM stages.
- `on_punt`: validates control traffic or learns a local source from a true no-route punt, then advertises readiness if appropriate.
- `on_timer`: expires neighbors, classifies customers, refreshes LSAs, retries coordination, sends HELLOs/anti-entropy, and schedules the next timer.

### Discovery and topology

- `handle_hello`: rejects stale/duplicate sequences, learns or revives a neighbor, repairs mistaken customer classification, replies, and synchronizes the LSDB.
- `refresh_local_lsa`: emits a new complete local snapshot only after a real local fact changes.
- `handle_lsa`: accepts only a strictly newer origin sequence, cancels obsolete coordination, and floods the new snapshot.
- `expire_neighbors`: implements the 100 ms silence test.
- `classify_customer_ports`: confirms data-bearing ports after 50 ms without a HELLO.
- `emit_anti_entropy`: rotates one LSA per quiet timer.

### Routing

- `mutual_graph`: includes only bidirectionally advertised edges.
- `distances_from`: BFS distances.
- `desired_routes`: deterministic next hops that decrease distance.
- `component_members` and `next_hop_to`: derive coordination membership and unicast paths from the same mutual graph.
- `apply_route_phase`: diffs only routes assigned to one distance phase; changed entries are deleted before installation.

### Ordered update

- `view_id` and `coordination_context`: identify the exact LSDB, current component, and lowest-ID leader.
- `maybe_announce_ready` / `handle_ready`: send, forward, retry, and collect exact-view readiness.
- `maybe_start_leader_update`: freezes desired routes only after every member is ready.
- `launch_current_phase`: applies the leader's local work, floods the phase, and begins ACK collection.
- `handle_update_phase`: validates exact context/generation, applies expected work, ACKs after actions, and refloods new attempts.
- `handle_phase_ack` / `maybe_advance_phase`: accept only the current phase and advance only after all component members ACK.
- `retry_coordination`: retries an incomplete phase every 25 ms.
- `mark_routing_change`: supersedes any in-progress update when a newer LSA changes the view.

## Development order used

### Part 1 foundation

1. Create the SDK crate and a minimal two-stage data plane.
2. Add bounded HELLO/LSA codecs.
3. Discover port roles without depending on port 100.
4. Learn observed customer addresses and flood complete LSAs.
5. Build a mutual graph and deterministic BFS routes.
6. Diff `/32` table entries safely.
7. Pass each Part 1 world before enabling failure logic.

### Part 2 base behavior

8. Add HELLO freshness and 100 ms neighbor expiration.
9. Immediately withdraw routes using a failed local port.
10. Keep probing down ports and synchronize recovered links.
11. Add rotating anti-entropy.
12. Run all 15 schedules and confirm delivery/reachability/recovery.

### Transient-loop hardening

13. Replay the two schedules that exposed transient mixed-FIB loops.
14. Add `ViewId`, READY barrier, leader generation, ordered distance phases, ACKs, and retries.
15. Remove the blind 5 ms SPF hold after the exact-view barrier made it redundant and slower.
16. Add stale-generation, duplicate/retry, phase-order, barrier, and supersession tests.
17. Rerun targeted logs, then the full matrix, then repository tests.
18. Measure no-route loss, recovery, control traffic, action safety, and WASM size.

## Testing order

1. Protocol/routing/state unit tests.
2. Release WASM build and formatting.
3. All five Part 1 practice worlds.
4. Targeted loop regressions: `part2-grid-003-f002` and `part2-dumb-006-f003`.
5. All 15 Part 2 schedules with unique log names.
6. Log-based loop/TTL/no-route inspection.
7. Rapid overlapping failure/recovery replay.
8. Complete repository/SDK/simulator suite.

Detailed commands and results are in `A1_TEST_PLAN.md` and `A1_IMPLEMENTATION_LOG.md`.

## Likely failure points and defenses

| Risk | Defense |
|---|---|
| Mistaking transit traffic for a local customer | Learn only on unknown/customer ports; HELLO overrides candidates |
| Old HELLO masking a dead link | Require strictly increasing HELLO sequence |
| One stale endpoint resurrecting a link | Require mutual LSA claims |
| Lost LSA | Immediate flood, recovery sync, rotating anti-entropy |
| Duplicate route entries | Stable IDs, unchanged suppression, delete-before-install |
| Mixed old/new FIB loop | Exact-view barrier and destination-outward phases |
| Lost phase or ACK | 25 ms attempts; duplicates re-ACK |
| Old phase overwriting new topology | Match view/leader/generation/phase and reject older generations |
| Change during an update | Cancel/freeze old generation and announce new view |
| Action output too large | Conservative batching; withhold ACK until all phase work fits |
| Sparse IDs or equal paths | Ordered maps and deterministic tie-breaking |
| Barrier liveness after failure | Membership comes from current mutual component; READY/phase retries |

## Final pre-submission sequence

The implementation itself is settled. Before submission:

1. Review the uncommitted diff and updated documents.
2. Rebuild the release WASM in the expected submission location.
3. Rerun at least the five Part 1 worlds and the two former loop regressions if any code changes.
4. If no code changes, preserve the current full-matrix evidence rather than redesigning.
5. Commit only when the user chooses, with no generated targets or logs.
