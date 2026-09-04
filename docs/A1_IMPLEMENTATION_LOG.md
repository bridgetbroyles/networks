# Assignment 1 implementation log

This is the evidence-oriented record of the implementation. It records what the code actually does, which files changed, which commands were run, and how failures were resolved. Proposed behavior belongs in `A1_DESIGN.md`; only verified results are called complete here.

## Current status

- **Part 1:** implemented and verified on all five supplied practice worlds.
- **Part 2:** implemented and verified on all 15 supplied failure schedules.
- **Simulator/grader changes:** none.
- **Git baseline:** planning commit `7094dd2` (`v1 planning`) on `main`; implementation remains uncommitted while it is being tested.

## Implementation resumed on 2026-09-04

The repository already contained an untracked `a1_switch/` crate when work resumed. It was inspected rather than recreated.

### Existing implementation found

`a1_switch/Cargo.toml` defines a standalone Rust 2024 `cdylib`/`rlib` crate using the provided SDK. The source is divided into:

- `src/lib.rs`: switch state, TinyVM setup, punt/timer handlers, discovery, route updates, and action budgeting;
- `src/protocol.rs`: a bounded, checked binary HELLO/LSA codec;
- `src/routing.rs`: mutual-adjacency graph construction, BFS distances, deterministic next-hop selection, and desired routes.

The implementation uses two exact-match tables. Protocol 253 is punted to the controller as control traffic; all other packets reach an exact destination `/32` route lookup. Each switch sends 25 ms HELLOs, waits 50 ms before classifying an unknown data-bearing port as a customer port, times a neighbor out after 100 ms, and applies a 5 ms route-computation hold. New complete LSAs are flooded and one rotating LSDB record is sent as periodic anti-entropy.

### Build and unit-test results

The host initially had no Rust toolchain. A temporary official Rust 1.98.1 toolchain and the `wasm32-unknown-unknown` target were installed under `/private/tmp`; this did not change the repository or the user's global Rust configuration.

The first Cargo attempt was blocked before compilation because the sandbox could not create `a1_switch/Cargo.lock`. Repository-scoped write permission resolved that environment issue.

The unchanged implementation then produced:

- `cargo test --manifest-path a1_switch/Cargo.toml`: **12 passed, 0 failed**;
- release WASM build for `wasm32-unknown-unknown`: **passed**;
- `cargo fmt --check`: formatting differences only; no semantic finding. Formatting is corrected before the next verification sweep.

The repository test suite passed all simulator/SDK tests except one run made with a redirected `CARGO_TARGET_DIR`. The failing test expects the learning-switch WASM at the repository's normal `target/` path, so it could not find that artifact. Re-running `cargo test --test wasm_integration` with the normal target location passed. No source or test was changed. Results observed before that environment-only failure were 64 library tests, 8 BGP integration tests, 9 simulator integration tests, 3 sim-log tests, and 2 trace-reply tests all passing; the separately rerun WASM integration test also passed.

### Part 1 practice-world results

The release WASM was run with the release simulator and the repository's `--score` report card.

| World | C1 scored delivery | C2 reachability | C3 loops | Result |
|---|---:|---:|---:|---|
| `practice-ring-002` | 100.00%, 0 lost | 30/30 | 0 | PASS |
| `practice-hub-009` | 100.00%, 0 lost | 42/42 | 0 | PASS |
| `practice-grid-003` | 100.00%, 0 lost | 90/90 | 0 | PASS |
| `practice-dumb-006` | 100.00%, 0 lost | 156/156 | 0 | PASS |
| `practice-ring-010` | 100.00%, 0 lost | 210/210 | 0 | PASS |

Each raw run received 98.90% of all generated packets because initial learning punts are dropped. Those packets occur before the 200 ms scored window; every scored Part 1 packet was delivered. The first ring run was saved as `/private/tmp/a1-part1-ring-002-before-fixes.simlog` for local replay inspection.

### Why Part 1 currently works

HELLOs arrive early enough to identify every inter-switch port before the 50 ms customer classification delay expires. Customer-origin packets identify each local host address, the resulting LSAs distribute all host locations, and mutual-adjacency BFS installs a route whose remaining distance decreases at each hop. By 200 ms all supplied topologies have converged. Deterministic tie-breaking and unchanged-route suppression keep tables stable, which is consistent with the zero-loop report cards.

## Files changed or generated

Intentional assignment work:

- `a1_switch/Cargo.toml`
- `a1_switch/Cargo.lock`
- `a1_switch/src/lib.rs`
- `a1_switch/src/protocol.rs`
- `a1_switch/src/routing.rs`
- `.gitignore`
- all `docs/A1_*.md` status/documentation updates

Cargo `target/` directories and `.simlog` files are ignored build/test artifacts. No file under simulator `src/`, grader/scoring code, SDK code, practice worlds, or failure schedules has been edited.

## Part 2 supplied-schedule results

All 15 world/schedule combinations pass the assignment's required criteria. C1 ranges from 99.38% to 99.59%, every C2 ordered pair is reached, and all 180 scored link events recover within 1,000 ms. The worst observed recovery is 148 ms.

| World family | Schedules passing | C1 range | Worst C4 recovery |
|---|---:|---:|---:|
| `part2-ring-002` | 3/3 | 99.44–99.56% | 144 ms |
| `part2-hub-009` | 3/3 | 99.41–99.48% | 140 ms |
| `part2-grid-003` | 3/3 | 99.43–99.52% | 143 ms |
| `part2-dumb-006` | 3/3 | 99.38–99.52% | 148 ms |
| `part2-ring-010` | 3/3 | 99.55–99.59% | 145 ms |

Thirteen schedules have no loop observation. `part2-grid-003-f002` and `part2-dumb-006-f003` each report six packets briefly revisiting a switch during asynchronous convergence; six and four packets respectively expire by TTL. The loops end as soon as the independently updated tables converge, do not affect C1/C2/C4 success, and are advisory in A1. Settled routes remain loop-free because every chosen next hop has a strictly smaller BFS distance to the customer origin.

The first parallel scoring attempt used the simulator's automatic score-log names. Runs for three schedules of the same world raced on that shared filename, producing decode/EOF errors after otherwise complete simulations. Rerunning with a unique `--log` path per schedule produced 15 valid report cards. No simulator change was made.

### Focused hardening after the sweep

The HELLO freshness comparison was tightened from “reject lower sequence” to “reject lower **or equal** sequence.” A duplicated HELLO can therefore no longer extend neighbor liveness. Tests were added for duplicate/stale HELLOs, timeout and recovery, stale/duplicate/higher LSAs, and delete-before-install route replacement.

## Remaining work

Required implementation work is complete on the supplied repository material. Final verification after the HELLO freshness change produced 15/15 native A1 tests, 5/5 Part 1 passes, 15/15 Part 2 passes, a successful conventional release WASM build, and a fully passing repository test suite (93 tests across the simulator, SDK, wire types, and integrations).

Remaining optional work is to inspect the two transient-loop replays for quiz understanding or add broader hidden-world stress generation. An ordered-update protocol is intentionally out of scope unless a required criterion fails. The instructor handout should also be checked for any rule not present in the repository, especially expectations for unobserved hosts inside a customer prefix.
