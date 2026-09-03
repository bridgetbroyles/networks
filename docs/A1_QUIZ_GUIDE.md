# Assignment 1 quiz guide (proposed design)

This guide explains the planned implementation in student-friendly terms. It must be revised after coding so every statement describes the **actual** submitted program.

## The one-sentence story

Every switch repeatedly asks, “Who is directly beside me?”, shares its local answer with the network, learns where customer addresses live, computes a shortest next hop, and changes that answer when a neighbor goes silent or comes back.

## The two halves of a switch

The data plane is the fast, simple half. For every packet it asks:

1. Is this one of our control packets? If yes, punt it to the controller.
2. Otherwise, is the destination address in the route table? If yes, send it through that port.
3. If neither table gives an egress port, punt it as `NoRoute`.

The control plane is the thinking half. It runs only when a packet is punted or its timer fires. It discovers facts, computes paths, and asks the data plane to install/delete entries. Those requests take a small simulated trip through the config pipe before taking effect.

## Vocabulary to be able to explain

- **HELLO:** a one-hop “I am switch X” packet sent repeatedly on possible switch links.
- **Neighbor:** a switch that has sent a valid HELLO on one local port recently.
- **LSA:** a complete, versioned statement from one switch listing its live neighbors and local customer addresses.
- **LSDB:** the collection of the newest LSA from every known switch.
- **Flooding:** sending a newly accepted LSA to every other live neighbor so it crosses the network.
- **Anti-entropy:** slowly re-sending known LSAs so one lost control packet is eventually repaired.
- **SPF/BFS:** shortest-path computation on an unweighted graph.
- **FIB/route table:** destination address to egress port entries used by the data plane.
- **Sequence number:** an increasing version that makes an old LSA lose to a newer one.
- **Liveness timeout:** how long the switch tolerates missing HELLOs before declaring a neighbor down.
- **Convergence:** the period from a topology change until switches have the new information and routes.

## What information is genuinely available

At initialization a switch gets only its own ID and the local port numbers with cables. It does not know the other endpoint of a port.

A punt gives the current simulated time, ingress port, reason, packet size, source/destination IP, protocol, TTL, and payload. It does not reveal the app ID, packet kind, path, or the customer's prefix length.

That last limitation matters. The practice files say customers own `/24`s, but the program only observes addresses such as `10.0.70.1`. The planned implementation advertises that exact address as a `/32`. This routes every destination the workload actually generates without pretending the API told us `/24`.

## Question type 1: “What happened?”

For this kind of question, narrate events in time order and separate observations from decisions.

### Example: startup on a line

```text
customer A -- S1 -- S2 -- S3 -- customer C
```

What happens:

1. The host calls `init` separately on S1, S2, and S3.
2. The first timer fires at time zero; every switch sends HELLOs.
3. S1 learns which port reaches S2. S2 learns ports to S1 and S3. S3 learns S2.
4. Early customer packets miss the empty route table and are punted. S1 observes A's source address; S3 observes C's source address.
5. After a port has customer traffic but no HELLO for the classification delay, that switch declares a local customer.
6. Each local change creates a newer LSA. Flooding gives all three switches the same topology/customer map.
7. Each switch computes shortest next hops and installs exact routes.
8. Later packets stay in the data plane and reach the correct customer.

The first punted packets are not rescued. They are startup learning evidence, and the 200 ms warmup keeps those expected losses out of the grade.

### Example: reading a report card

If C1 is 99.95%, C2 is 90/90, C3 says zero loops, and C4 says 11/12 events within budget, the run fails because C4 is a required criterion. A high whole-run average does not hide one slow recovery.

If C1 and C2 pass but C3 reports two loops, A1 still passes because C3 is advisory. The loops are still bugs worth understanding and would waste capacity in a later assignment.

## Question type 2: “Why did it happen?”

For a why question, connect the observed symptom to one piece of state and one rule.

### Why did a control packet reach `on_punt`?

Its protocol matched the initial control-table entry. That entry's action is `Punt`, so route lookup did not forward it as customer traffic.

### Why did an ordinary packet get punted?

No route-table entry matched its destination, so the pipeline finished without an egress. The simulator converted that into `PuntReason::NoRoute`.

### Why did S1 choose S2 instead of S3?

Both were live direct neighbors, but only a neighbor one hop closer to the destination is eligible. If both had the same remaining distance, the lower neighbor switch ID won the deterministic tie-break.

### Why was an old LSA ignored?

The LSDB already had the same origin with a sequence number greater than or equal to the received sequence. Accepting it would restore stale links or customers.

### Why is a link usable only when both ends advertise it?

One endpoint may have heard a HELLO while the other has not yet done so, or one endpoint may have timed out first. Mutual agreement prevents routing over that half-known link.

### Why delete before installing a changed route?

The simulator appends table entries; matching entry IDs are not automatic replacements. Deleting first prevents an older route to the same destination from remaining in front of the new one.

### Why did recovery take about 100–125 ms?

The link failed silently. The program waited until HELLO silence crossed the 100 ms timeout, advertised the change, then waited for the next periodic computation after the short stability hold. Nothing directly told it “link down.”

### Why might a link coming back cause a C4 row?

Restoration is scored as an event too. A bad program can create a transient loop or black hole while switching back to the shorter route, so the scorer measures UP as well as DOWN.

## Question type 3: “What would happen if?”

For a hypothetical, identify which event fires, which state changes, which message is sent, and how the route table follows.

### If a HELLO is lost once

Nothing dramatic happens. Other HELLOs arrive every 25 ms, and the neighbor is not declared down until 100 ms of silence. One loss is tolerated.

### If all HELLOs on S1--S2 stop

Both endpoints eventually time out the adjacency. Each originates a newer LSA without the neighbor. Once either side's new LSA reaches the rest of the network, the mutual-link rule removes S1--S2 from path computation. Routes move to an alternate path if the graph remains connected.

### If S1--S2 comes back

Both switches have continued sending HELLOs on the down port. A HELLO now crosses, the port becomes live again, and new LSAs add the adjacency. The link is used only after both endpoint claims are known.

### If an old “link is up” LSA arrives after a newer withdrawal

It is ignored because its origin sequence is smaller. It cannot resurrect the failed link.

### If the same new LSA arrives twice

The first copy is installed and flooded. The equal-sequence second copy is ignored, stopping flood loops.

### If an LSA is lost during flooding

The immediate change may still reach the switch through another graph path. If not, rotating anti-entropy later sends the missing origin's latest LSA across each direct adjacency.

### If two equal-length paths exist

The eligible next hops both reduce distance by one. The program chooses the lower switch ID (then lower port) so repeated computations are stable. Either choice is a valid shortest path.

### If a customer packet arrives on a known switch-neighbor port with no route

The packet is lost as a punt, but its source is **not** learned as a local customer. Learning it locally would create a false customer location and potentially a loop or misroute.

### If a port carries customer traffic before its first HELLO arrives

The source becomes only a candidate. The switch waits 50 ms before classifying the port. A valid HELLO during that window makes it a switch port instead.

### If a timer is not rescheduled

That switch receives no more timer callbacks. HELLOs stop, neighbors eventually think its links are down, and it cannot detect later failures or recoveries.

### If `on_timer` returns too many actions

If their postcard encoding exceeds 4,096 bytes, the call fails and that switch's controller is never called again. Its existing data-plane routes freeze. That is why anti-entropy sends one LSDB record per tick instead of all records in every packet.

### If a route is installed twice without a delete

Both entries remain. Depending on table order/priority, the old port can continue winning, and repeated updates can fill the table.

### If a packet briefly loops during convergence

Every switch ingress decrements TTL, so it eventually dies rather than looping forever. The design's stability hold, mutual links, and failed-next-hop withdrawal aim to avoid this; after LSDB convergence, shortest-path distance strictly decreases and a persistent loop is impossible.

## Small topology drills

### Triangle failure

```text
        S2
       /  \
      S1--S3
```

Customer C3 is attached to S3. Initially S1 may route directly to S3. If S1--S3 fails:

- S1's last HELLO from S3 grows old.
- S1 withdraws the direct adjacency and advertises a newer LSA.
- The settled graph says S1's distance to S3 is two.
- S2's distance is one, so S1's C3 route changes to the port toward S2.
- At S2 the route goes directly to S3. Distances go `2 -> 1 -> 0`, so the settled path cannot loop.

### Diamond tie

```text
       S2
      /  \
    S1    S4 -- customer D
      \  /
       S3
```

S1 has two length-two paths to S4. If S2 has a lower ID than S3, S1 selects S2. If S1--S2 fails, S3 becomes the only neighbor with distance one to S4, so the route changes to S3.

### Stale update

Suppose S2's LSA sequence 8 lists S4, and sequence 9 removes S4. If sequence 9 arrives first and delayed sequence 8 arrives later, every receiver keeps 9. “Latest arrival” is not enough; the origin's sequence number decides freshness.

## A useful answer framework

For almost any quiz exhibit, answer in four sentences:

1. **Observation:** “S2 stopped receiving HELLOs from S3 on port 1.”
2. **State change:** “At 100 ms of silence it marked that neighbor down and incremented its LSA sequence.”
3. **Propagation/computation:** “The new LSA flooded, the mutual graph removed S2--S3, and BFS chose S1 as the next hop.”
4. **Data-plane effect:** “The controller deleted the old destination entry and installed the same destination on the port to S1.”

If the question concerns an exact timestamp, remember the config and punt pipes, link latency/serialization, periodic timer phase, and the fact that the timer delay begins when the scheduling action arrives—not exactly when the previous handler started.

## Facts worth memorizing

- One program instance per switch; private state is not shared.
- Startup knowledge: own ID plus local port IDs only.
- Wakeups: punts and requested timers only.
- Link failures and restorations are silent.
- Punt-visible packet fields are a strict subset of TinyVM-visible fields.
- Timer and punt time is integer nanoseconds.
- Route installs/deletes are delayed config actions.
- Table install appends; it does not replace.
- Packet TTL is decremented before each switch pipeline run.
- Warmup is 200 ms; tail guard is 100 ms.
- Part 1 delivery floor is 99.9%; Part 2 is 98%.
- Every DOWN and UP event has a 1,000 ms recovery budget.
- C2 requires every ordered app pair.
- C3 loops are advisory in A1, not permission to ignore them.

