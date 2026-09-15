# Assignment 1 design: implemented architecture

This document describes the code currently in `a1_switch/`. The design has been validated on all five supplied Part 1 worlds, all 15 supplied Part 2 schedules, and a separate rapid-change replay.

## Architecture at a glance

Every switch runs the same Rust/WASM program. It uses link-state routing plus a small ordered-update protocol.

- The TinyVM data plane punts reserved control packets and otherwise performs one exact destination lookup.
- The Rust control plane discovers neighbors with HELLOs and learns local customer addresses from ordinary no-route punts.
- Each switch floods a versioned LSA containing its live neighbors and local customers.
- All switches compute deterministic shortest paths over mutually advertised links.
- Before changing FIBs, switches agree on an identical routing view. The lowest-ID switch in the connected component coordinates distance-ordered update phases.
- Missing HELLOs detect a silent failure. HELLOs continue on down ports, so their return detects recovery.

Link state is a good fit because A1 is one administrative network with at most 15 switches, unit-cost links, no routing policy, and a complete topology small enough to replicate everywhere.

## Simulator contract that drives the design

1. `init(switch_id, local_ports)` runs once per switch. It reveals only the switch's ID and attached local port numbers, not remote endpoints or port roles.
2. The control plane runs only for `PuntEvent` and requested `TimerEvent` callbacks.
3. `PuntEvent` exposes time, switch and ingress port, punt reason, packet size, source/destination IP, IP protocol, TTL, and payload. It does not expose app identity, packet kind, path, or customer prefix length.
4. `TimerEvent` exposes only simulated nanoseconds. Timer requests are relative and travel through the configuration pipe.
5. Injected control packets use a chosen local egress port and consume ordinary link/queue capacity.
6. Table installs and deletes are asynchronous configuration actions. Install appends; it does not replace an entry with the same ID. A changed route must therefore be deleted before replacement.
7. A failed A1 link silently drops new enqueues. Already queued packets may drain, and no link event is delivered to the student program.
8. Recovery is silent too. Only traffic that starts crossing again reveals it.
9. A1 links are bidirectional, with shared FIFO/drop-tail serialization. Switch ingress decrements packet TTL before TinyVM runs.
10. The controller output limit is 4,096 encoded bytes per callback and the WASM instruction limit is 4,000,000 per callback. Table and TinyVM limits are also bounded.
11. Practice traffic starts at time zero, sends every 10 ms, and rotates destinations. Scoring excludes the first 200 ms and final 100 ms.
12. C1 requires 99.9% Part 1 or 98% Part 2 delivery, C2 requires every ordered app pair, and C4 requires recovery within 1,000 ms after every DOWN and UP event. C3 reports loops and is advisory for A1.

A punted customer packet cannot be rescued by an SDK action; learning helps later packets. Customer prefix length is not observable, so the implementation learns and routes each observed application address as `/32`.

## Data plane

There are two exact tables and two TinyVM stages:

```text
ip_proto -> T_CONTROL -> punt protocol 253
ip_dst   -> T_ROUTE   -> set egress port
```

`T_CONTROL` has one initial entry. Control payloads also require the `NA1!` magic, version 1, and a valid bounded encoding before they affect state. `T_ROUTE` has capacity 256 and uses the destination address as a stable entry ID. A route miss becomes `PuntReason::NoRoute`.

## Control-plane state

Each switch owns private state:

- per-port role, neighbor ID/liveness, last HELLO time and sequence, and candidate customers;
- local HELLO and LSA sequence numbers;
- latest LSA per origin (`lsdb`) and local customer-to-port mappings;
- installed routes, including egress port and logical next-hop switch;
- anti-entropy cursor and `routing_dirty`;
- current 128-bit `ViewId`, readiness sequence, and readiness records;
- active ordered update: leader, generation, view, maximum/completed phase, and desired routes;
- leader-only update state: component members, current phase/attempt, acknowledgements, and last-send time;
- newest generation seen per leader and newest flooded attempt per generation/phase.

`BTreeMap`/`BTreeSet` avoid assuming dense switch IDs and make identical inputs produce identical encodings, hashes, leader choices, and paths.

## Control messages

All messages use a checked binary format and carry a common magic/version/type header.

- `HELLO(sender, sequence)`: one-hop identity and liveness probe.
- `LSA(origin, sequence, neighbors, customers)`: complete replacement snapshot for one origin.
- `READY(sender, leader, sequence, view)`: states that the sender has exactly this LSDB view and is ready to update it.
- `UPDATE_PHASE(leader, generation, view, phase, max_phase, attempt)`: starts or retries one ordered FIB phase.
- `PHASE_ACK(sender, leader, generation, view, phase, attempt)`: confirms that the sender finished all local actions for that phase.

HELLO and LSA lists are canonicalized. Decoders reject malformed lengths and more than 64 list items before allocation. LSAs and phases flood across live neighbor ports. READY and ACK messages follow deterministic shortest paths toward the leader, reducing overhead.

## Discovery and LSDB maintenance

HELLOs are sent every 25 ms on unknown, live-switch, and down-switch ports. A valid new HELLO binds an ingress port to its neighbor. Equal or older HELLO sequence numbers cannot refresh liveness.

An unknown port that supplies ordinary customer traffic becomes a candidate customer port. It is confirmed only after 50 ms without a HELLO. A later HELLO overrides that classification and withdraws mistaken local-customer state.

Each local neighbor/customer change increments the local LSA sequence and floods a complete snapshot. A receiver accepts only a strictly newer sequence for that origin. One rotating LSDB record per quiet timer provides anti-entropy; a newly discovered/recovered neighbor also receives a direct database sync.

Routing admits edge A--B only when A's latest LSA names B and B's latest LSA names A. This prevents a one-sided or stale adjacency from carrying traffic.

## Route computation

For every advertised customer address, BFS computes unit-cost distances from its owner over the mutual graph. A switch chooses a live direct neighbor whose distance is exactly one less than its own, breaking ties by neighbor ID and then local port. A local customer has distance zero and uses its customer port.

For one settled view, every forwarding hop strictly lowers distance:

```text
d, d-1, d-2, ... , 1, 0
```

Therefore the final FIB for a common view is loop-free. If two LSAs impossibly claim the same customer, the lowest origin ID wins deterministically.

## Why ordinary independent FIB updates were insufficient

The original implementation installed each switch's new shortest path as soon as that switch learned an LSA. Two switches could temporarily hold routes computed from different views. After a topology change, A could start forwarding to B while B still forwarded to A. The final paths were valid, but packets present during this mixed interval could revisit a switch or die by TTL. The supplied `grid-f002` and `dumb-f003` replays each exposed six such packets.

A fixed quiet-time hold reduced timing differences but could not prove that every switch had the same information or installed dependent routes in the correct order. The current protocol explicitly coordinates both facts.

## Routing views and agreement barrier

A `ViewId` is a deterministic 128-bit digest of every ordered LSDB record: origin, sequence, neighbor list, and customer list. Matching IDs mean the switches have the same routing input, up to negligible hash-collision risk.

For its mutual-graph component, each switch chooses the lowest switch ID as leader and immediately sends READY for its current view. The leader starts only when every member it derives from that same view has sent READY for exactly that view. A view change clears active coordination and causes a new READY. Old-view messages then fail validation.

This is an agreement barrier, not consensus: the simulator's deterministic, bounded A1 setting does not require durable elections or fault-tolerant replicated state. If a link splits the control graph, each resulting connected component derives its own membership and lowest-ID leader after the relevant LSAs arrive.

## Ordered FIB phases

After the barrier, the leader chooses a monotonically increasing generation and floods phases from 0 through `component_size - 1`.

- Phase 0 deletes destinations now unreachable and installs/updates local distance-0 routes.
- Phase 1 applies routes whose new distance is 1.
- Phase 2 applies routes whose new distance is 2.
- Later phases continue outward to the component's maximum possible distance.

Each switch snapshots desired routes when it accepts the generation. It performs all route actions for a phase before injecting its ACK. Config actions and the ACK share preserved configuration-pipe ordering, so receipt of an ACK implies the route changes were submitted first. The leader advances only after every member ACKs.

The safety intuition is destination-rooted: a distance-`d` switch changes only after all distance-`d-1` switches have completed the previous phase. Its new next hop is therefore already prepared. Unreachable old routes are removed first, preferring a brief no-route drop to forwarding into a loop. The official matrix observed 74 such drops across 15 long failure runs, all transient; every destination route returned.

## Loss, duplication, supersession, and liveness

- READY is retransmitted every 25 ms while routing is dirty.
- The leader retries an unacknowledged phase every 25 ms with a larger `attempt`.
- A node re-ACKs duplicate phases it has already completed. A higher attempt is reflooded, repairing a lost phase or ACK path.
- A full action batch withholds the ACK. On retry, already-applied actions need no space, so remaining actions and then the ACK can progress.
- A newly accepted LSA immediately cancels active/leader update state and clears phase-flood state. Coordination restarts for the new view.
- A phase older than `latest_generation[leader]` is rejected. View, leader, generation, phase, component size, and expected phase must all match before a FIB change occurs.
- Delayed ACKs can only satisfy the leader's exact active generation/view/phase.

Thus a delayed message from an old topology cannot overwrite newer routing state. Repeated real topology changes can postpone completion by repeatedly superseding work, but once changes stop the retries guarantee progress as long as the component communicates. A rapid-change replay with changes 5--85 ms apart converged with zero loops and a 50 ms worst measured recovery.

## Failure and recovery behavior

The key timings are:

| Mechanism | Value |
|---|---:|
| HELLO/timer interval | 25 ms |
| Customer classification | 50 ms |
| Neighbor dead interval | 100 ms |
| READY/phase retry | 25 ms |

On failure, each endpoint eventually sees 100 ms of HELLO silence, marks the neighbor down, immediately deletes installed routes using that failed local port, and originates a newer LSA. Mutual adjacency can disappear as soon as either newer endpoint claim is learned. Once component members agree, the ordered phases install alternate paths.

On recovery, continued probes cross again. A fresh HELLO marks the port live, replies immediately, syncs the LSDB, and originates a new LSA. The edge is eligible only after both endpoints advertise it; a new view then installs shorter paths in order.

## Resources and measured cost

- Two tables, two TinyVM stages, no registers/counters, and at most one route per observed customer (15 in supplied worlds).
- Routing work is comfortably bounded at A1 scale. BFS is `O(V+E)` per customer.
- Action batching reserves timer space and caps estimated output at 3,200 bytes, below the 4,096-byte hard limit.
- The largest official topology and every update phase ran without output, fuel, table-capacity, action-budget, crash, or panic failures.
- Release WASM: 204,194 bytes versus 177,327 bytes at the passing checkpoint (+15.2%).
- Across all 15 Part 2 logs, control ingress-hop count rose from 792,295 to 985,890 (+24.4%) and hop-bytes from 37,356,302 to 50,722,440 (+35.8%). This is about 1.08 Mb/s aggregated over all network links and runs, negligible beside 1 Gb/s links.

The added complexity and traffic bought zero observed loops, 22.6% fewer losses overall, and equal or faster worst recovery on every supplied schedule.

## Important assumptions and remaining risks

- A1's documented bound is 15 switches and supplied worlds remain connected during each single-link failure. The code leaves modest headroom but is not a general Internet routing protocol.
- Customer prefix length is unobservable; exact observed-host routing matches the simulator's generated destinations. A separate rule requiring arbitrary unseen addresses inside a prefix would need an API source for that prefix.
- `ViewId` uses two deterministic 64-bit hashes, not a mathematical collision-free LSDB serialization. Collision risk is negligible for A1 but not zero in theory.
- A permanently failed controller cannot ACK. Its incident links will eventually disappear when neighbors time it out; until the new component view forms, the barrier intentionally waits rather than risk a mixed update.
- Continuous topology churn can continuously supersede generations. Official schedules leave at least 1,000 ms between changes; the extra rapid-change replay also converged, but an adversary that never stops changing the graph cannot be promised convergence.
- Brief black holes remain an intentional safety tradeoff during phase 0 and physical link loss.

No simulator, scorer, SDK, world, or failure-schedule source is modified by this implementation.
