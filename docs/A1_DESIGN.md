# Assignment 1 design (pre-implementation)

This document records the proposed design for Assignment 1. It is deliberately a plan, not an implementation. After the switch is built, this file and `A1_QUIZ_GUIDE.md` should be checked against the actual code and updated wherever the implementation differs.

## Executive summary

Each switch will run the same small link-state routing program.

- The TinyVM data plane will punt one reserved control protocol and otherwise perform one exact destination-address lookup.
- The Rust controller will discover switch neighbors with periodic one-hop `HELLO` messages.
- It will learn locally attached customer addresses from ordinary packets arriving on ports that never answer a hello.
- Every switch will originate a versioned link-state advertisement (LSA) containing its currently live switch neighbors and its locally learned customer addresses.
- New LSAs will be flooded hop by hop. A slow rotating anti-entropy exchange will repair a missed advertisement without creating a control storm.
- Each switch will compute deterministic unit-cost shortest paths over links that both endpoints currently advertise, then install one route per learned customer address.
- Missing hellos will withdraw a link; continued probes will rediscover a restored link. Sequence numbers reject stale LSAs.

This is intentionally not BGP, spanning-tree forwarding, or a distance-vector protocol. A1 is a single-owner network of at most 15 switches with no routing policy. A complete topology database is small, shortest paths are easy to explain, and a 100 ms liveness timeout is far inside the 1,000 ms recovery budget.

## What the simulator actually exposes

The repository, especially [`switch-programs.md`](switch-programs.md), [`switch_program_sdk/src/lib.rs`](../switch_program_sdk/src/lib.rs), [`switch_program_types/src/lib.rs`](../switch_program_types/src/lib.rs), [`sim.rs`](../src/sim.rs), [`world.rs`](../src/world.rs), and [`score.rs`](../src/score.rs), establishes the following contract.

1. `init(switch_id, local_ports)` runs once for every owned switch when the WASM module is installed. It returns both the TinyVM program and all table/register/counter declarations.
2. At startup the program knows its own `u32` switch ID and the sorted, deduplicated `u16` IDs of ports with links. It is not told the remote endpoint, which ports face customers, the topology, customer IDs, IPs, or prefix lengths.
3. The controller executes only on a punt or a timer. The CLI schedules one initial timer at simulation time zero. Every later timer must be requested by the program.
4. `PuntEvent` contains exactly: `now_ns`, `switch_id`, `ingress_port`, `reason`, `packet_size`, `ip_src`, `ip_dst`, `ip_proto`, `ip_ttl`, and the opaque `payload` bytes. It does **not** expose packet kind, DSCP, transport ports, app ID, flow ID, custom fields, labels, packet ID, or the hidden path trail.
5. `TimerEvent` contains only `now_ns`.
6. `ScheduleTimer { delay_ns }` is a relative delay. The request itself traverses the config pipe; the new timer is scheduled relative to the time the action arrives, so a nominal periodic timer drifts slightly.
7. `InjectPacket` chooses an attached local egress port and supplies source/destination IP, protocol, TTL, kind, size, and payload. The SDK's `inject_packet` helper creates a `Data` packet whose nominal size is 20 bytes plus payload. Injection traverses the config pipe and then the ordinary link, consuming bandwidth.
8. Control messages are ordinary packets. The receiver gets them only if TinyVM punts them. Their payload is opaque to TinyVM and readable by `on_punt`.
9. Customer traffic in the supplied worlds uses IP protocol 0. The proposed program reserves protocol 253, a non-customer destination address, a magic value, and a message version for its own control traffic. A first-stage exact table punts that protocol before destination routing. `PuntEvent` does not contain `PacketKind`, so protocol plus validated payload is the reliable controller-visible discriminator.
10. TinyVM tables are ordered vectors. Exact tables try entries by descending priority; LPM tables use longest prefix then priority. The first match applies its action immediately. A miss simply falls through. If the whole pipeline ends without an egress, the simulator punts with `NoRoute`.
11. Tables and their capacities are declared in `init`; initial entries may be added there. Later installs/deletes are asynchronous controller actions through the config pipe. Installing an entry appends it; it does not replace an existing entry with the same ID. Deleting an ID removes every entry with that ID. The implementation must therefore diff routes, delete a changed entry before reinstalling it, and never reinstall unchanged routes.
12. A punt reveals a packet's source and destination addresses but not either prefix length. In the supplied worlds, every app owns a `/24` but sends from and receives at the prefix's first host. The only topology-independent fact learnable through the student API is the observed host address. The design advertises and installs that address as `/32`; that is sufficient for the actual workload builder and avoids inventing an unknowable prefix length.
13. A `HELLO` payload can identify the switch at the far end of the ingress port. There is no direct neighbor query.
14. Hard limits are 4,000,000 WASM instructions per call; 4,096 encoded output bytes per call; 16 tables with 1,024 entries each; 16 register arrays and 16 counter arrays with 4,096 slots each. TinyVM's default validator limit is 64 instructions per stage and one distinct memory resource per stage.
15. Time is deterministic simulated `Duration`, exposed to WASM as integer nanoseconds in `u64`. Events are ordered by time and then scheduling sequence. Link and pipe serialization delays use integer nanoseconds.
16. Every physical link is bidirectional but has one shared FIFO/drop-tail serialization queue. Packets preserve enqueue order. A switch ingress decrements TTL before running TinyVM; a packet arriving with TTL zero is dropped. World links are 1 Gbps, 100 microseconds, and 65,536-byte queues unless overridden. A1 worlds do not override capacity.
17. When a link fails, new enqueues in either direction are silently dropped. Packets already enqueued still arrive. `notify_link_events` defaults to false, A1 leaves it false, and the Rust SDK intentionally does not expose `on_link_event`.
18. A restored link also generates no student-visible event. It can be detected only because continuously transmitted probes begin arriving again.
19. A world assigns inter-switch ports densely from zero in link declaration order and attaches an app at port 100. Every practice A1 world has one app per switch. The design nevertheless discovers port roles and does not depend on port 100, dense switch IDs, or published topology order.
20. The report card scores `Data` packets whose source and destination are both declared app addresses. It ignores sends before 200 ms and in the final 100 ms. C1 is delivery (99.9% Part 1, 98% Part 2), C2 requires every ordered app pair to deliver at least once, C3 reports loops but is advisory in A1, and C4 requires the last loss attributed to every DOWN and UP event to be no later than 1,000 ms after that event.

Two source-level details are worth remembering:

- A punted customer packet is already lost. Installing a route helps later packets; there is no SDK action that forwards the original packet with its identity intact.
- The world sends one packet per app every 10 ms and rotates across destinations. It does not send to every peer every 10 ms.

## Data plane

The proposed data plane has two small exact-match tables and two stages.

### Control table

`T_CONTROL` is exact-match on `ip_proto`. It has one initial entry for protocol 253 whose action is `Punt(Custom(CONTROL_REASON))`. This prevents a control packet from accidentally matching a customer route.

### Route table

`T_ROUTE` is exact-match on `ip_dst`. Each installed entry maps one observed customer address to an egress port. A miss leaves no egress and therefore generates `PuntReason::NoRoute`.

The pipeline is conceptually:

```text
stage 0: ip_proto -> T_CONTROL
stage 1: ip_dst   -> T_ROUTE
```

No registers, counters, labels, recirculation, queue changes, or trace machinery are needed. The switch's automatic ingress TTL decrement supplies loop protection.

## Control-plane state

Each WASM instance will maintain bounded Rust collections rather than arrays indexed by switch ID.

- `switch_id` and `local_ports` from `init`.
- Per-port state: role (`Unknown`, `SwitchNeighbor`, or `Customer`), neighbor ID if known, last accepted hello time, liveness, first ordinary packet time, and candidate customer addresses.
- A monotonically increasing local hello sequence.
- A monotonically increasing local LSA sequence.
- The latest accepted LSA per origin switch.
- The local map from customer address to customer-facing port.
- The last time topology/customer information changed and a `routing_dirty` flag.
- The desired and currently installed route for each customer address.
- A rotating cursor used to send one LSDB record per periodic anti-entropy round.

All lists placed on the wire will be sorted and deduplicated. Deterministic collections and tie-breaking make identical topology views produce identical decisions.

## Control messages

The program will use a small checked binary format rather than expose Rust memory layouts.

### Common header

- fixed magic bytes
- format version
- message type

Any packet with the reserved protocol but a wrong magic, version, length, or count is ignored safely.

### HELLO

Fields: immediate sender switch ID and hello sequence.

HELLO is sent with TTL 1 on every unknown or switch-facing local port. It is not sent on a confirmed customer port. Receipt associates `ingress_port` with the sender, records the neighbor as live, and refreshes `last_hello_ns`.

### LSA

Fields: origin switch ID, origin sequence number, complete sorted list of currently live neighbor switch IDs, and complete sorted list of locally attached customer `/32` addresses.

An LSA is a replacement snapshot, not an incremental add/remove. A receiver accepts only a sequence number strictly newer than its stored one, then floods the unchanged LSA on live switch ports except the ingress port. Equal and older LSAs are ignored. Self-origin LSAs received from the network are ignored.

When a new or recovered adjacency appears, each endpoint immediately sends its own current LSA and a bounded database sync over that port. During ordinary operation, each timer sends one rotating LSDB record on every live switch port. With 15 origins and a 25 ms tick, every direct neighbor is refreshed within about 375 ms even if an earlier flood packet was lost.

## Startup and neighbor/customer discovery

At the first timer (time zero), the switch sends HELLO on all local ports and schedules the next timer.

An ordinary punt on a known switch-facing port is never treated as evidence of a local customer. On an unknown port, its source address becomes a candidate. The port is classified as customer-facing only after candidate traffic has been observed and the port has remained without a valid HELLO for 50 ms. This is long compared with the sub-millisecond delivery of a practice-world hello and short compared with the 200 ms scoring warmup.

If a HELLO later arrives on a port tentatively considered customer-facing, switch-neighbor evidence wins: candidate customer state on that port is withdrawn and a new local LSA is originated. This makes the classifier recover from a startup race instead of locking in a mistake.

When a customer port is confirmed, every observed source address on it is added to the local customer map. The local LSA is replaced with a new sequence. Practice traffic begins at time zero and repeats every 10 ms, so every customer address should be observed well before scoring begins.

## Building the topology

The LSDB describes directed claims: origin `A` says that `B` is live. Routing treats `A-B` as usable only when both the latest LSA from `A` lists `B` and the latest LSA from `B` lists `A`. This mutual-adjacency rule avoids routing over a half-discovered or one-sided stale link.

Customer address ownership is derived from the customer list in each latest LSA. The supplied worlds guarantee disjoint prefixes and one app per switch. If malformed information claims the same address at multiple origins, the implementation will choose the lowest origin switch ID so every switch with the same LSDB resolves the conflict identically.

## Route computation

For every advertised customer address:

1. If its origin is this switch, use the recorded local customer port.
2. Otherwise run breadth-first search on the mutually advertised graph, because all A1 links have equal routing cost.
3. Choose a live local neighbor whose distance to the destination origin is exactly one less than this switch's distance. Break ties by the smallest neighbor switch ID, then smallest local port ID.
4. If no such path exists, the address has no desired route.

The strict distance decrease proves that routes computed from one settled LSDB cannot loop: every hop reduces the remaining hop count. It also gives deterministic shortest paths on unseen graphs and arbitrary switch-ID numbering.

The controller diffs desired routes against `installed_routes`:

- unchanged route: no action;
- new route: install once;
- changed route: delete its stable entry ID, then install the new action;
- unreachable/withdrawn address: delete the old entry.

Actions are emitted in delete-before-install order because the simulator preserves config-pipe action order. This creates only a serialization-scale miss window and avoids the duplicate-entry behavior of table installs.

## Failure detection and recovery

Proposed timing constants:

| Mechanism | Value | Reason |
|---|---:|---|
| HELLO/timer interval | 25 ms | Fast detection with negligible bandwidth at A1 scale. |
| Customer-port classification delay | 50 ms | Gives two hello opportunities and still converges before the 200 ms warmup. |
| Neighbor dead interval | 100 ms | Four nominal hello periods; tolerant of small scheduling drift and far below the 1,000 ms budget. |
| SPF stability hold | 5 ms minimum, checked on the next periodic timer | Longer than normal diameter-wide control propagation, while usually adding at most one 25 ms tick. |

At each timer, a live neighbor whose most recent HELLO is at least 100 ms old is marked down. The switch removes that neighbor from its local LSA, increments the sequence, and floods the replacement. A route whose own next-hop port just died is withdrawn immediately; the complete SPF diff waits until topology information has been stable for at least 5 ms and a timer runs. This favors a short black hole over bouncing a packet back toward an upstream switch.

HELLO transmission continues on down neighbor ports. After link restoration, the first received HELLO marks the adjacency live, increments the local LSA, triggers direct LSDB synchronization, and eventually makes the edge usable once both endpoint LSAs agree.

Expected practice-world bounds are roughly:

- failure: at most about 100–125 ms to detect and commit new routes, plus sub-millisecond propagation/config time;
- restoration: normally under 50 ms to hear a hello, exchange LSAs, and commit.

These are comfortably inside the report card's 1,000 ms limit and leave room for conservative behavior.

## Stale information and loop prevention

The design uses several independent safeguards.

- Origin sequence numbers make duplicate and out-of-order LSAs harmless.
- Complete replacement LSAs remove stale neighbors and customers rather than accumulating them.
- Mutual adjacency prevents one stale endpoint claim from resurrecting a link.
- Periodic anti-entropy repairs a missed flood and synchronizes a recovered edge.
- A short SPF hold lets an LSA cross the tiny network before FIBs change.
- Deterministic shortest paths strictly decrease distance after convergence.
- Failed next-hop routes are deleted before an alternate is installed.
- Ingress TTL decrement bounds any transient inconsistency that does occur.

The design guarantees no persistent forwarding loop after the LSDB settles. Like ordinary distributed link-state routing, it cannot make all switches update atomically, so a very short transient loop is theoretically possible while config-pipe updates arrive. The hold and break-before-make behavior make that window small; the report card also treats C3 as advisory in A1, though the implementation should still aim for zero loop observations.

## Resource use

- Two tables, no register arrays, no counter arrays.
- At most one route per customer address (15 in documented A1 scale) in a route table sized for 256 entries.
- Two TinyVM stages with two instructions and one table access each.
- Controller state is on the order of `O(V + E + customers)`; SPF is at worst `O(customers * (V + E))` for `V <= 15`.
- An immediate LSA flood produces at most one packet per live neighbor in one handler.
- A normal timer emits at most one HELLO plus one anti-entropy LSA per switch-facing port, plus one timer request. Anti-entropy is skipped on a timer that has a large route diff. This keeps the postcard-encoded action vector comfortably below 4,096 bytes even at high degree.
- Control packets are small and the practice workload uses only a tiny fraction of 1 Gbps. Control traffic is not scored, but it still uses the shared links.

All decoders and loops will have explicit bounds. No handler will wait, recurse, or iterate on data supplied without a size check.

## Alternatives considered

### Distance vector

It needs less topology state, but stale advertisements and count-to-infinity/loop-avoidance rules make failure behavior harder to implement and explain. With only 15 switches, its main advantage is irrelevant.

### One spanning tree

It is simple in steady state, but loses shortest paths and must rebuild when a tree edge fails. Safe distributed tree replacement is not simpler than flooding LSAs here.

### Flooding customer data

TinyVM has no multicast action, controller reinjection does not preserve the original workload packet's identity, and flooding would waste bandwidth and risk duplicates. It does not fit the API or scorer.

### Full LSDB in every periodic packet

It converges, but high-degree switches could exceed the 4,096-byte returned-action limit. Immediate delta flooding plus one-record-per-tick anti-entropy is bounded and sufficient.

### Learning `/24` from practice files

All published apps use `/24`, but `PuntEvent` never reveals that length. Advertising the observed `/32` host address is more honest and remains correct for every destination the actual workload generator produces.

## Assumptions and open uncertainties

- The repository itself states in `README.md` that the instructor handout is not included. No handout file exists in this checkout or the task's initial directory. This plan therefore relies on the user's stated requirements plus the executable scorer. The handout should be checked before implementation for any rule not encoded here, especially whether students are expected to forward every address within a customer prefix rather than only workload destinations.
- The docs say the TinyVM program is validated after `init`, but the current `load_program`/`install_program` path does not visibly call `tinyvm::validate`. The planned TinyVM is valid under the documented limits and will not rely on this discrepancy.
- General simulator controllers have `on_link_event`, but A1 disables notifications and the SDK trait deliberately omits it. The design does not rely on it.
- The practice schedules contain one failed link at a time and guarantee connectivity. The protocol can process multiple independent link losses if the remaining graph stays connected, but that is not the primary tested promise.
- Switch/CPU failures are implemented by the simulator but do not appear in A1 failure schedules. This design addresses link failures, not a permanently failed node or controller.

