# Assignment 1 implementation plan (no code yet)

This plan translates the architecture in `A1_DESIGN.md` into a Rust/WASM switch program. None of these program files have been created in Phase 1.

## Files to create or modify in the implementation phase

Create a standalone crate outside the simulator workspace, following the examples:

```text
a1_switch/
  Cargo.toml
  src/
    lib.rs          SwitchProgram implementation, timers, port state, action assembly
    protocol.rs     checked HELLO/LSA encoding and decoding
    routing.rs      graph construction, BFS, deterministic next-hop selection, route diff
```

Optional test-only files may be added under `a1_switch/tests/` if integration tests are easier to keep separate. The existing simulator, SDK, scorer, worlds, failure schedules, and their tests must not be modified.

`a1_switch/Cargo.toml` should opt out of the parent workspace with `[workspace]`, build a `cdylib`, use edition 2024, point to `../switch_program_sdk`, and use the small release profile from `examples/hello_switch`.

The only Phase 1 changes are these documentation files:

- `docs/A1_DESIGN.md`
- `docs/A1_IMPLEMENTATION_PLAN.md`
- `docs/A1_QUIZ_GUIDE.md`
- `docs/A1_TEST_PLAN.md`

## Constants

Define constants in one place and explain each one:

- table IDs for control and routes;
- control IP protocol 253 and a custom punt reason;
- control destination/source address convention;
- wire magic and version;
- `HELLO_INTERVAL_NS = 25_000_000`;
- `PORT_CLASSIFY_NS = 50_000_000`;
- `NEIGHBOR_DEAD_NS = 100_000_000`;
- `SPF_HOLD_NS = 5_000_000`;
- route-table capacity 256;
- decoder limits (for example, no more than 64 neighbors or customer addresses in one LSA).

All timing decisions must use `TimerEvent.now_ns` and `PuntEvent.now_ns`, not a count of timer callbacks, because config-pipe delivery makes periodic timers drift.

## Major types

### `A1Switch`

Proposed fields:

- `switch_id: u32`
- `ports: BTreeMap<u16, PortState>`
- `hello_seq: u64`
- `local_lsa_seq: u64`
- `lsdb: BTreeMap<u32, Lsa>`
- `local_customers: BTreeMap<u32, u16>`
- `installed_routes: BTreeMap<u32, InstalledRoute>`
- `routing_dirty: bool`
- `last_routing_change_ns: u64`
- `sync_cursor: usize`

Avoid vectors indexed by switch ID; hidden worlds may use sparse IDs.

### `PortState`

Proposed fields:

- role enum: `Unknown`, `Switch { neighbor_id, live }`, or `Customer`;
- `last_hello_ns: Option<u64>`;
- `first_data_ns: Option<u64>`;
- sorted candidate customer addresses.

A HELLO is stronger evidence than tentative customer classification. A down switch port retains its neighbor ID so probes continue and recovery is recognizable.

### `Lsa`

- `origin: u32`
- `sequence: u64`
- sorted `neighbors: Vec<u32>`
- sorted `customers: Vec<u32>`

It is a complete snapshot for one origin.

### `InstalledRoute`

- stable `entry_id: u64` derived from the destination address;
- destination address;
- egress port;
- optionally the chosen next-hop switch for diagnostics/tests.

### Protocol enums

Use an internal `ControlMessage::{Hello, Lsa}` enum after decoding. The wire decoder should never panic on short, oversized, unknown-version, or trailing data.

## Important functions

### Setup and pipeline

- `A1Switch::init(switch_id, local_ports)`: create all per-port states; declare both tables; install the control punt entry; parse/set the two-stage TinyVM; seed the local LSDB record.
- `build_setup()`: keep table and TinyVM declarations isolated so their IDs cannot drift apart.

### Message handling

- `protocol::encode_hello(...)` and `decode_control(...)`.
- `protocol::encode_lsa(...)` with sorted lists and checked length fields.
- `handle_hello(now, ingress_port, hello, actions)`: refresh liveness, detect new/recovered/change-of-identity adjacency, rebuild the local LSA, flood it, and send bounded database synchronization to a new neighbor.
- `handle_lsa(now, ingress_port, lsa, original_bytes, actions)`: require a known live switch-facing ingress, reject self/equal/older LSAs, replace the origin record, mark routing dirty, and flood to other live neighbors.
- `handle_customer_punt(now, ingress_port, ev)`: record only the source address, and only on an unknown or customer port. Never learn a customer from a known switch-facing port.

### Local state and flooding

- `rebuild_local_lsa(now) -> Option<Lsa>`: compare the complete local neighbor/customer snapshot with the current one; increment sequence only on change.
- `flood_lsa(lsa, except_port, actions)`: one TTL-1 control injection per live switch-facing port except the ingress.
- `sync_neighbor(port, actions)`: send the current local record immediately and then a bounded subset of the LSDB. Remaining records are supplied by rotating anti-entropy ticks.
- `emit_periodic_control(now, actions)`: HELLO every unknown/down/live switch port and, when the action budget permits, one rotating LSA record to each live neighbor.

### Failure/recovery

- `expire_neighbors(now, actions)`: mark a switch neighbor down when `now - last_hello_ns >= NEIGHBOR_DEAD_NS`; immediately delete installed routes whose next hop is that port; rebuild/flood the local LSA.
- `classify_customer_ports(now, actions)`: after the quiet interval, promote unknown ports with candidate addresses and originate a new LSA.
- Recovery occurs in `handle_hello`; there is no link-event handler in the SDK.

Use saturating subtraction or explicit ordering for time comparisons. A malformed or unexpected event must be ignored, not unwrapped.

### Routing

- `routing::mutual_graph(lsdb)`: include an undirected edge only if both endpoint LSAs name one another.
- `routing::customer_origins(lsdb)`: produce deterministic address-to-origin mappings.
- `routing::distances_from(origin, graph)`: BFS on sorted adjacency.
- `routing::desired_routes(self_id, ports, local_customers, lsdb)`: local port for local customer, otherwise a live neighbor with distance exactly one lower; tie-break on neighbor then port.
- `apply_route_diff(desired, actions)`: emit no action for unchanged routes; install new; delete removed; delete then install changed. Update `installed_routes` to the state that will result after ordered config actions.
- `maybe_recompute(now, actions)`: run only when dirty and topology has been stable at least `SPF_HOLD_NS`.

Keep graph and route computation pure so native unit tests can exercise them without the simulator.

### Handler shape

`on_punt` should:

1. If protocol/reason identify control traffic, decode and dispatch HELLO/LSA.
2. Otherwise record possible local-customer evidence.
3. Return only bounded immediate flooding/sync actions. Do not schedule the periodic timer here.

`on_timer` should:

1. expire silent neighbors;
2. classify quiet customer ports;
3. rebuild/flood the local LSA if local facts changed;
4. recompute/diff routes if the stability hold has elapsed;
5. emit HELLO and a bounded anti-entropy slice;
6. emit exactly one next `ScheduleTimer` action.

The action assembler should suppress optional anti-entropy during a large LSA or route update and have a conservative maximum action count. The required timer action must never be omitted.

## Entry IDs and table update rules

Use one deterministic route entry ID per IPv4 destination, for example `u64::from(ip)`, in the route table. Table IDs provide a separate namespace, so this does not conflict with the control table's initial entry.

The simulator's install is append-only, even for an already used ID. Therefore:

- never reinstall an unchanged route;
- for a changed route, emit `delete_entry` before `install_route`;
- for a withdrawal, delete and remove it from `installed_routes`;
- do not assume an install replaces an old entry;
- size the route table far above the documented maximum of 15 customers, but below the 1,024-entry limit.

## Implementation order

### Foundation

1. Create the standalone crate and copy only the structural pattern from `hello_switch`.
2. Implement the two-table TinyVM setup and constants.
3. Implement protocol encode/decode with native unit tests, including malformed input.
4. Implement per-port state and HELLO neighbor mapping.

### Part 1 first

5. Record customer source addresses on non-neighbor ports after the classification delay.
6. Implement full replacement LSAs, sequence checks, and immediate flooding.
7. Implement mutual graph construction, deterministic BFS, and desired routes.
8. Implement route diffs and exact `/32` table entries.
9. Run every Part 1 world until C1, C2, and C3 are clean. Do not add failure logic while steady-state discovery is uncertain.

### Part 2 second

10. Add hello timestamps and the 100 ms dead-neighbor transition.
11. Add local LSA withdrawals, immediate invalid-next-hop deletion, and the SPF stability hold.
12. Add recovery handling and direct LSDB synchronization.
13. Add rotating anti-entropy and confirm the action vector stays below 4,096 bytes.
14. Run all 15 supplied failure schedules and inspect every DOWN and UP row.

### Hardening

15. Test duplicate, stale, out-of-order, truncated, and oversized control messages.
16. Test sparse switch IDs, tie paths, high-degree nodes, and multiple customer candidates without changing grader code.
17. Review every `unwrap`, index, loop bound, and return size to prevent fatal WASM traps/fuel/output failures.
18. Update the design and quiz guide to describe the code exactly.

## Testing order

Use the detailed matrix in `A1_TEST_PLAN.md`. The short order is:

1. native protocol/routing unit tests;
2. WASM release build;
3. smallest Part 1 ring;
4. all five Part 1 worlds;
5. one failure schedule per topology family with `.simlog` inspection;
6. all 15 schedules;
7. repeated deterministic runs and final full sweep.

## Likely failure points

- Forgetting to reschedule the timer, which stops all liveness work.
- Treating every `NoRoute` punt as a local customer and learning remote sources on transit ports.
- Assuming `/24` when only a host address is observable.
- Updating an existing route by installing again, leaving duplicate entries and possibly selecting the old port.
- Using the published numeric switch IDs or port 100 as topology knowledge.
- Considering a link live after only one endpoint advertises it.
- Refreshing liveness on arbitrary LSAs instead of HELLO, which can hide a dead direct adjacency.
- Accepting equal/older LSAs or failing to advertise a complete replacement after a withdrawal.
- Stopping HELLOs on a down port, making recovery impossible to detect.
- Recomputing routes directly inside every LSA punt and creating unnecessary transient inconsistency.
- Generating full-database floods in one callback and exceeding 4,096 output bytes.
- Parsing untrusted payload slices with unchecked indices or counts.
- Assuming timer callbacks occur exactly every 25 ms rather than comparing `now_ns`.
- Reading C1 alone: a high average can hide a C2 black hole or a C4 event over budget.

## Completion criteria for the implementation phase

The implementation is ready only when:

- all five Part 1 report cards pass C1 and C2, ideally with zero C3 loops;
- all 15 Part 2 schedules pass every C4 event, including restorations;
- no program-failure message, integrity warning, persistent loop, or baseline steady-state loss appears;
- route-table logs show no duplicate entries and no stale failed-port route after convergence;
- the docs have been updated from proposed to actual behavior;
- Git status contains only intentional program, test, and documentation changes.

