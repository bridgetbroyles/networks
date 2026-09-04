# Assignment 1 test plan and results

This document combines the test strategy with verified results. Part 1 and Part 2 supplied matrices are complete. Command-level details and failures are recorded in `A1_IMPLEMENTATION_LOG.md`.

## Test objectives

Testing must establish more than a passing whole-run percentage:

- complete discovery before the 200 ms scored window;
- delivery for every ordered app pair;
- stable, loop-free forwarding when all links are healthy;
- sub-1,000 ms response to every failure and every restoration;
- rejection of stale, duplicate, malformed, and out-of-order control data;
- no duplicate/stale forwarding entries;
- no WASM trap, fuel exhaustion, or output-size failure;
- bounded, understandable control overhead.

## Environment preflight

Before implementation testing:

1. Confirm `cargo`, `rustc`, and `rustup` are available.
2. Install/check `wasm32-unknown-unknown`.
3. Build the simulator in release mode.
4. Run the repository's existing test suite before student-code changes.
5. Confirm Git is clean except for intentional documentation/program work.

A temporary Rust 1.98.1 toolchain with `wasm32-unknown-unknown` was provisioned under `/private/tmp`. The simulator release build, the A1 native tests, the A1 release WASM build, and the repository's tests all pass when run in their expected target locations. The test suite's hard-coded normal target-path assumption is documented in `A1_IMPLEMENTATION_LOG.md`.

## Unit tests inside the program crate

### Protocol codec

Test round trips for smallest and largest expected HELLO/LSA messages. Test rejection of:

- wrong magic or version;
- unknown message type;
- truncated header or list;
- declared count larger than remaining payload;
- count beyond the configured maximum;
- trailing garbage, if the format requires exact consumption.

The decoder must return `None`/an error, never panic.

### Freshness rules

- first LSA for an origin is accepted;
- higher sequence replaces it;
- equal sequence is ignored and not reflooded;
- lower sequence is ignored;
- received self-origin LSA is ignored;
- a replacement snapshot removes a neighbor/customer absent from the new list.

### Port classification

- HELLO maps the exact ingress port to the sender.
- Ordinary traffic on an unknown port creates a candidate, not immediate customer state.
- HELLO before the classification deadline prevents customer classification.
- A known switch port never learns a transit packet's source as local.
- A customer port can learn its observed source address.
- A later HELLO can correct a tentative misclassification.

### Topology graph

- one-sided adjacency is excluded;
- mutual adjacency is included;
- withdrawal from either endpoint removes the edge;
- sparse/non-zero-based switch IDs work;
- disconnected input yields no route rather than a panic.

### Routing

Use line, triangle, diamond, ring, and disconnected graphs.

- Local customer route uses the local port.
- Remote route takes a shortest path.
- Every selected neighbor has distance exactly one less to the destination.
- Equal-cost tie-break is deterministic.
- No route is produced without a path or known local neighbor port.
- Duplicate customer claims resolve deterministically.

For every small generated graph, follow the chosen next hops and assert that distance strictly decreases until the origin. This directly tests the no-persistent-loop invariant.

### Route diffs

- empty to populated emits one install per address;
- unchanged emits nothing;
- changed port emits delete then install;
- withdrawn address emits delete;
- mixed changes preserve deterministic action order;
- repeated recomputation does not grow the table.

### Timing state

- one missing HELLO does not fail a neighbor;
- 100 ms of silence does;
- comparisons use event times despite timer drift;
- down ports keep receiving outgoing HELLO probes;
- recovered HELLO changes liveness and originates a newer LSA;
- SPF waits for the stability hold.

## Native simulator/integration tests

Build small in-memory topologies or temporary world files without changing grader code.

### Startup timeline

Log a three-switch line from 0–250 ms. Success means:

- HELLOs identify all inter-switch ports;
- only genuine access ports learn customers;
- all customer LSAs reach all switches;
- route entries are installed before 200 ms;
- losses occur only during warmup;
- no route duplicates appear in the viewer.

### Silent failure

On a triangle, fail the direct link used by one route at a non-timer-aligned time. Success means:

- no `on_link_event` dependency;
- the local endpoint keeps trying HELLOs;
- the link is removed after the liveness timeout;
- a replacement path is installed well inside 1,000 ms;
- traffic remains stable afterward.

### Recovery

Restore that link at a jittered time. Success means:

- a later HELLO, not a schedule assumption, detects recovery;
- both endpoint LSAs must advertise it before use;
- routes may return to the shorter path;
- no persistent loop or stale failed-port route remains.

### Control-message disorder

Where practical, inject LSAs directly into a controller test in orders such as `5, 7, 6, 7`. The final stored version must be 7 and flooding must be bounded. Repeat an accepted LSA many times and verify no action storm.

### Resource/high-degree case

Use a 15-switch high-degree topology. Record the maximum encoded handler output and approximate fuel. Success means every call remains comfortably below 4,096 bytes and 4,000,000 instructions; optional anti-entropy is skipped when route/flood work is large.

## Part 1 practice matrix

Every world uses 1 Gbps links, 100 microsecond latency, 65,536-byte queues, 10 ms per-app traffic, a 200 ms warmup, a 10,000 ms run, and a 99.9% delivery floor.

| World | Shape | Switches / links / apps | Diameter | What it stresses | Success |
|---|---|---:|---:|---|---|
| `practice-ring-002.toml` | ring with chords | 6 / 9 / 6 | 2 | Small first end-to-end case; equal paths | C1/C2 pass; C3 ideally zero; no baseline loss |
| `practice-hub-009.toml` | dual-hub/spoke | 7 / 11 / 7 | 2 | High-degree ports and deterministic ties | Same, with no output-size issue |
| `practice-grid-003.toml` | grid | 10 / 13 / 10 | 5 | Longer LSA spread and several equal paths | Same; all 90 ordered pairs reachable |
| `practice-dumb-006.toml` | dumbbell | 13 / 15 / 13 | 6 | Sparse cut-like structure and long routes | Same; all 156 ordered pairs reachable |
| `practice-ring-010.toml` | large ring/chords | 15 / 18 / 15 | 6 | Maximum documented state and route count | Same; all 210 ordered pairs reachable, no resource warnings |

Run the smallest world first, then sweep all five. A correct Part 1 report should show:

- C1 at or very near 100%, above 99.9%;
- C2 exactly `N*(N-1)` reached;
- C3 zero revisits/TTL deaths if possible;
- C4 `n/a` because no link event occurred;
- no `WHERE PACKETS STOPPED`, `WHAT WENT WRONG`, `INTEGRITY`, or program-failure section for scored traffic.

Verified on 2026-09-04: all five worlds meet these conditions. Every C1 result is 100.00% with zero scored losses, C2 is complete (30, 42, 90, 156, and 210 ordered pairs respectively), and every C3 result is zero.

Inspect startup separately with a shorter run/log; a perfect unscored total is not expected because learning punts are discarded.

## Part 2 failure matrix

Part 2 twins have the same graphs but a 60,000 ms nominal world duration and 98% delivery floor. When a schedule is supplied, its roughly 23–27 second duration wins. Every schedule has six non-overlapping failure intervals, therefore 12 scored state-change events, and a 1,000 ms recovery budget.

Run all three schedules for every topology:

| World | Schedules | Main stress |
|---|---|---|
| `part2-ring-002.toml` | `f001`, `f002`, `f003` | Frequent equal-cost alternatives in a small graph |
| `part2-hub-009.toml` | `f001`, `f002`, `f003` | High-degree hubs; every published blast radius is 1 |
| `part2-grid-003.toml` | `f001`, `f002`, `f003` | Multiple alternate paths and mixed blast radii |
| `part2-dumb-006.toml` | `f001`, `f002`, `f003` | Large blast radii up to 20 and diameter 6 |
| `part2-ring-010.toml` | `f001`, `f002`, `f003` | 15-switch scale, diameter 6, blast radius up to 21 |

For every run, success means:

- C1 at least 98%;
- C2 every ordered app pair reached;
- C4 `12/12 events recovered within 1000 ms`;
- each recovery-timeline row says `ok`;
- no steady-state `baseline_loss` outside event windows;
- no controller failure;
- no persistent loop after convergence;
- restored links are actually rediscovered and may be selected again.

Do not test only failures with small blast radius. The dumbbell and large-ring schedules are especially valuable because one wrong next hop affects many source/destination pairs.

Verified on 2026-09-04: all 15 schedules pass C1, C2, and C4. C1 ranges from 99.38% to 99.59%; all 180 DOWN/UP events recover within budget; the worst observed recovery is 148 ms. Thirteen schedules have zero C3 loops. `part2-grid-003-f002` and `part2-dumb-006-f003` each observed six transient revisits during convergence, but no persistent loop and no required-criterion failure.

## Focused behavior checks

### Normal forwarding

Pick one source/destination pair and inspect its route at every hop. The chosen next hop must be live and one distance closer to the customer's origin. The final switch must select its customer port.

### Warmup

Compare logs at 25, 50, 100, and 200 ms. By 200 ms, all customers and routes should be present. If not, do not hide the problem by increasing the scorer's warmup override.

### Topology discovery

For every switch, reconstruct the expected neighbor-to-port mapping from the world declaration order and compare it with HELLO observations in debug output/log-derived behavior. The program itself must not use that expected mapping.

### Customer discovery

Verify that each switch advertises its app's first-host address and that no switch advertises a remote source as local. Confirm the route table uses exact host keys.

### Link failure

Measure:

- failure event to last accepted in-flight HELLO;
- last HELLO to local timeout;
- timeout to LSA arrival at farthest switch;
- LSA arrival to route-table change;
- event to last lost workload packet (the report-card value).

### Route changes

Inspect table logs around a changed route. There should be one delete followed by one install, not accumulating entries. Unchanged destinations should not be rewritten.

### Link recovery

Confirm HELLOs were attempted while the link was down, the first post-restore HELLO is observed, both endpoint LSAs advance, and the mutual graph admits the edge only after both claims arrive.

### Stale and duplicate information

Unit/integration tests should show old LSAs cannot overwrite new ones, duplicates are not reflooded, and periodic anti-entropy can restore a deliberately omitted record.

### Loops

Use C3 plus packet traces. Any repeated switch after the recovery window is a correctness bug. A transient repeat should trigger inspection of the SPF hold and update ordering even though C3 is advisory.

## Report-card interpretation

- **C1 failure:** too many scored packets were lost. Separate baseline loss from failure-attributed loss.
- **C2 failure:** at least one ordered pair never delivered. Start with the named pair and example path; often one customer origin or local route is missing.
- **C3 note:** a packet revisited a switch or died by TTL. Inspect mixed old/new routes and stale next hops.
- **C4 failure:** at least one DOWN or UP event's last loss was more than 1,000 ms after the event. Use the exact row, link, and example packet.
- **Program failed:** fix this before interpreting routing symptoms. The data plane freezes at the moment of a trap/fuel/output error.
- **Integrity warning:** do not forge workload-like packets and verify the run/world/schedule combination.

Remember that C4 defines recovery as the time of the **last lost packet sent** after an event, not the first subsequent success. With round-robin traffic, the measurement is quantized by when an affected source/destination pair next sends.

## Logs and replay viewer

For any suspicious run, keep a named `.simlog` rather than relying on the temporary score log. Use the browser viewer to:

- pause just before and after a failure;
- inspect each switch's current route table;
- confirm the failed link rendering;
- follow a report-card example packet;
- look for repeated hops;
- verify delete/install order and restored-link reuse.

The generic log does not include control payload bytes, so detailed HELLO/LSA decoding may require temporary, bounded debug state/tests. Do not modify the simulator log schema for the assignment.

## Reproducible verification command sequence

After choosing `PROGRAM` as the built WASM path:

```sh
cargo test --all-targets

./target/release/competitive_net_sim run-world \
  worlds/practice/practice-ring-002.toml \
  --program "$PROGRAM" --score --log /tmp/a1-part1.simlog

for world in worlds/practice/practice-*.toml; do
  ./target/release/competitive_net_sim run-world \
    "$world" --program "$PROGRAM" --score
done

for schedule in worlds/practice/failures/*.toml; do
  stem=$(basename "$schedule" | sed -E 's/-f[0-9]+\.toml$//')
  ./target/release/competitive_net_sim run-world \
    "worlds/practice/${stem}.toml" \
    --failures "$schedule" --program "$PROGRAM" --score
done
```

The final sweep was rerun from the final release build with a unique log path per run. Unique paths matter when schedules for one world run concurrently because automatic score-log names are world-based. Local logs under `/private/tmp/a1-final-*.simlog` preserve all five Part 1 and 15 Part 2 runs for quiz review during this session.

## Hidden-world risk tests

Before submission, add local tests for conditions not strongly represented by the five published graphs:

- sparse and shuffled switch IDs;
- a 15-switch high-degree node;
- several equal-cost routes;
- a link failing just before/after a timer;
- restoration just before/after a HELLO attempt;
- one dropped immediate LSA flood packet;
- repeated stale LSA delivery;
- malformed control payloads;
- multiple simultaneous link losses while the graph remains connected (best-effort robustness, even if not promised);
- a customer prefix length other than `/24`, confirming exact first-host routing still serves generated workloads.

The key hidden-world defense is to derive everything from observed ports, HELLOs, LSAs, and packet addresses rather than published IDs, filenames, topology families, failure times, or port 100.
