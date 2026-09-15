# Assignment 1 quiz guide: actual implementation

This guide explains the networking and distributed-systems ideas behind the final code. A strong quiz answer should connect an observed event to control-plane state, protocol action, and the resulting data-plane behavior.

## The short story

Every switch discovers its neighbors and local customer, shares those facts as link-state advertisements, and computes shortest paths. Before FIBs change, all switches in the current connected component prove that they have the same LSDB view. The lowest-ID switch then installs routes in distance order from each customer outward, waiting for acknowledgements after every phase. This prevents the temporary two-switch loops that independent updates caused.

## Data plane versus control plane

The data plane handles each packet quickly:

1. Protocol 253 matches the control table and is punted.
2. Otherwise, exact destination address matches the route table and sets an egress port.
3. A miss has no egress, so the simulator punts `NoRoute`.

The Rust control plane runs only on a punt or timer. It learns facts, sends control messages, computes routes, and returns table/timer/packet actions. Those actions take effect through a simulated configuration pipe; they are not instantaneous.

## Vocabulary to explain in your own words

- **HELLO:** repeated one-hop message saying which switch sent it.
- **Neighbor timeout:** 100 ms without a fresh HELLO means the link is treated as down.
- **LSA:** one switch's complete, sequence-numbered snapshot of live neighbors and local customer addresses.
- **LSDB:** the newest accepted LSA from every known origin.
- **Mutual adjacency:** A--B is usable only if A names B and B names A.
- **BFS distance:** number of unit-cost links remaining to a customer's switch.
- **FIB:** the fast exact destination-to-egress table.
- **Routing view:** the complete LSDB input used for one routing decision.
- **View ID:** a deterministic 128-bit digest of that complete LSDB.
- **Barrier:** the leader waits until every component member says READY for exactly the same view.
- **Generation:** the leader's increasing ID for one ordered update of one view.
- **Phase:** which new BFS distance may update now; phase 0 withdraws unreachable routes, phase 1 updates direct neighbors, then phase 2, and so on.
- **ACK:** proof that one switch submitted all actions for the current phase.
- **Supersession:** a newer LSA cancels an older update before the new view starts.
- **Anti-entropy:** slowly re-sending LSAs to repair a lost flood.

## Facts the program actually knows

At `init`, a switch knows only its own ID and its local port IDs. It is not told neighbors, topology, customer port, application IDs, IP prefixes, or failures.

A punt reveals time, ingress port, reason, size, IP source/destination/protocol/TTL, and payload. It does not reveal the app identity, hidden path, packet kind, or destination prefix length. Therefore the code learns observed customer host addresses and installs exact `/32` entries; it cannot honestly infer an arbitrary `/24` from the API.

Failures and recoveries are silent. The only failure evidence is missing HELLOs; the only recovery evidence is that HELLOs cross again.

## Question type 1: “What happened?”

Answer in time order. Separate a physical event, observation, state change, protocol exchange, and FIB effect.

### Startup on a line

```text
customer A -- S1 -- S2 -- S3 -- customer C
```

1. `init` creates empty route state and the two data-plane tables on each switch.
2. Time-zero timers send HELLOs. S1/S2/S3 learn which local ports lead to switches.
3. Early customer packets miss and are punted. Their source addresses become candidates only on non-switch ports.
4. After 50 ms with no HELLO, those ports are classified as customer ports.
5. Local changes create newer LSAs, which flood hop by hop.
6. Each switch hashes its ordered LSDB and sends READY toward the lowest-ID member, S1.
7. When S1 has READY from S1, S2, and S3 for the same view, it starts a generation.
8. Phase 0 installs local customer routes. Phase 1 installs routes one switch away; phase 2 installs routes two away. Every phase waits for all ACKs.
9. Later customer packets stay in the data plane and follow exact destination entries.

Early learning punts are already lost, but the 200 ms warmup excludes them. All five official Part 1 worlds deliver 100% of scored packets.

### A silent failure

Suppose S1--S2 fails at time T.

1. No notification reaches either program; packets and HELLO enqueues on that link begin dropping.
2. At about T plus 100 ms, each endpoint's timer finds the last HELLO too old.
3. The endpoint marks the neighbor down, removes routes using that local port immediately, and originates a higher-sequence LSA without the neighbor.
4. Accepting any new LSA cancels an in-progress old generation.
5. Once all members have the same new LSDB, READY messages complete the barrier.
6. Phase 0 removes now-unreachable routes. Later phases install alternate paths from destinations outward.
7. Delivery resumes; the worst official last-loss time was 145 ms, well inside 1,000 ms.

### Reading the final report card

`99.64%, 90/90, 0 loops/0 TTL, 12/12 events, worst 98 ms` means:

- some packets were expected to hit the failed link or a deliberate short withdrawal;
- every ordered source/destination pair still succeeded;
- no packet revisited a switch;
- all six downs and six ups recovered within budget;
- the slowest event's last attributed loss was 98 ms after the event.

## Question type 2: “Why did it happen?”

### Why did transient loops happen before the change?

Shortest-path routing is loop-free only when switches use a consistent view. During convergence, S4 might install “send to S5” from the new graph while S5 still has “send to S4” from the old graph. Each route was reasonable for its own view, but the pair together formed a loop. The checkpoint exposed this in two official schedules.

### Why does a quiet-time delay not fully solve that?

A timer delay guesses that information has spread; it does not prove it. Different LSAs, queueing, control loss, and callback timing can still make switches act on different inputs. READY messages compare the actual view instead of guessing from elapsed time.

### Why hash the whole LSDB?

One LSA sequence is not enough: a view contains the newest record for every origin. The deterministic 128-bit ID includes every origin, sequence, neighbor, and customer. The leader accepts READY only when that complete identity matches its own.

### Why is the lowest-ID switch the leader?

Every member with the same mutual graph can independently compute the same answer without election messages. It is deterministic and simple. Leadership is scoped to the current component and view.

### Why update small distances first?

A distance-3 route sends to a distance-2 switch. The distance-2 switch must already know its safe new downstream route before distance 3 redirects traffic to it. Updating outward from distance 0 establishes that dependency. ACKs tell the leader when a whole layer is ready.

### Why is phase 0 special?

An unreachable destination has no new distance. Keeping its old route could send traffic into a dead link or mixed-state loop, so phase 0 removes it first. A short black hole is safer than looping traffic and is bounded by later phases/recovery.

### Why does ACK ordering matter?

The program returns route actions before the injected ACK action. The simulator preserves configuration-pipe action order. Therefore, when the ACK leaves, the switch's relevant table work was submitted earlier; the leader does not move upstream too soon.

### Why can a completed phase be ACKed again?

The first ACK might be lost. A leader retries after 25 ms with a larger attempt. The switch sees that the phase is already complete, avoids reinstalling unchanged routes, and sends another ACK. This makes loss recoverable without rolling state backward.

### Why can old control messages not overwrite a new route?

A phase must match the current view, derived leader, component size, generation, and expected phase. `latest_generation` rejects an older generation. The leader accepts an ACK only for its exact current generation/view/phase. A new LSA clears active coordination. A delayed old phase therefore fails before applying a FIB change.

### Why require both endpoint LSA claims?

One endpoint may detect failure or recovery earlier. A single claim could route onto a half-known link. Mutual adjacency waits for consistent endpoint evidence.

### Why delete before installing a changed table entry?

The simulator appends installs rather than replacing by ID. Delete-then-install prevents stale duplicate entries. Both actions are kept together in one phase batch or deferred together.

## Question type 3: “What would happen if?”

### One HELLO is lost

Nothing changes. HELLOs arrive every 25 ms, and four nominal periods must be silent before the 100 ms timeout. Equal or delayed old sequences cannot fake freshness.

### An LSA flood is lost

Another graph path may carry it. If not, one rotating LSDB record per quiet timer eventually repairs it. A recovered neighbor also gets a direct database sync.

### A phase or ACK is lost

The leader stays on the same phase because not every member ACKed. After 25 ms it sends a higher attempt. New attempts reflood; completed switches re-ACK. The leader cannot skip the missing member.

### A link recovers during an old phase

A fresh HELLO revives the port and produces a new local LSA. Every receiver that accepts the new LSA cancels the old active update and announces the new view. Old phase/ACK messages no longer match. After both endpoint claims spread, a fresh generation installs the recovered paths.

### Another failure occurs before convergence

The newer LSA supersedes the unfinished generation in the same way. Work may restart more than once, but after topology changes stop, READY and phase retries make progress. The rapid-change replay exercised changes 5--85 ms apart with zero loops.

### The old leader becomes disconnected

After neighbor timeouts and LSAs remove the failed adjacency, each surviving mutual component recomputes membership. Its lowest remaining ID becomes leader. Messages from the old view or leader fail the context checks.

### A route phase exceeds the action budget

The switch applies only actions that fit and does not ACK the phase. On retry, already completed route diffs are unchanged and need no action, leaving room for remaining work and the ACK. Timer space is reserved separately.

### Two shortest paths tie

Both eligible next hops reduce distance. The lowest neighbor switch ID, then lowest port ID, wins. Determinism prevents needless route churn.

### A no-route customer packet arrives on a switch-neighbor port

It is lost, but it is not learned as a local customer. Otherwise one transient routing miss could falsely move a remote customer's location.

### A customer-looking packet arrives before the first HELLO

Its source is only a candidate. The 50 ms classification period gives HELLOs time to prove that this is actually a switch port. A later HELLO overrides a mistaken classification.

### A timer is not rescheduled

That switch stops sending HELLOs, stops expiring neighbors, and stops retries/anti-entropy. Other switches eventually consider its adjacencies down. Every timer callback therefore includes another timer action.

## Small topology drills

### Triangle reroute

```text
        S2
       /  \
      S1--S3 -- customer C
```

Initially S1 may use direct S1--S3. If that link fails, the new view gives distances: S3=0, S2=1, S1=2. Phase 0 handles unsafe withdrawals, phase 1 prepares S2's direct route to S3, and only phase 2 redirects S1 to S2. S1 never relies on an unprepared S2.

### Why independent updates can loop

```text
customer C -- S0 ... S4 -- S5
```

Imagine an old path makes S5 send toward S4, but the new path makes S4 send toward S5. If S4 changes first, the mixed FIB is S4 <-> S5. With ordered phases, whichever switch is closer in the new view is updated and acknowledged first; the farther switch redirects only later.

### New view supersedes old view

Suppose generation 7 for view X is in phase 2. A new LSA creates view Y. The switch clears X's active state and announces READY(Y). A delayed phase-3(X) fails the view check. When the leader has READY(Y) from all members, it starts a new generation using routes computed from Y.

## A reusable four-part answer

1. **Observation:** “S2 received no fresh HELLO from S3 for 100 ms.”
2. **State:** “S2 marked that port down, changed its local LSA, and canceled the old generation.”
3. **Coordination:** “The LSA flooded; members agreed on a new view; the leader ran withdrawal and distance phases with ACKs.”
4. **Data plane:** “The old entry was deleted and the alternate entry was installed only after its downstream dependency was ready.”

For exact timestamps, mention periodic timer phase, link latency/serialization, punt/config pipes, and round-robin traffic. C4 measures the last lost packet's send time, so it is quantized by when affected pairs send.

## Facts worth memorizing

- One private program instance per switch.
- Startup input: own ID and local ports only.
- Wakeups: punts and self-requested timers.
- HELLO 25 ms; customer classification 50 ms; dead neighbor 100 ms; retry 25 ms.
- Link down/up is silent.
- Control traffic: protocol 253 plus validated `NA1!` payload.
- LSAs are complete snapshots; strictly newer sequence wins.
- Routes use mutual links and exact observed customer `/32`s.
- Final next hops strictly decrease BFS distance.
- Leader: lowest ID in the current mutual component.
- Barrier: every member READY for one exact view.
- Phase 0 withdraw/local; later phases increase new distance.
- Every member must ACK before the leader advances.
- New LSA supersedes old coordination; stale generation cannot install.
- Part 1 final: 5/5, 100% scored delivery.
- Part 2 final: 15/15, zero loops/TTL, 180/180 events, worst 145 ms.
- Ordered implementation reduced official losses by 22.6% versus the checkpoint despite extra coordination.
