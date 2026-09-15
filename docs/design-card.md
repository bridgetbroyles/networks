# NetworkArena A1 Design Card

## The basic idea

Our switch program uses link-state routing. Each switch discovers its directly connected switch neighbors, learns which customer address is attached locally, and shares those facts with the rest of the network. Once the switches agree on the same topology, they compute shortest paths and install exact destination-address routes in the data plane.

The extra piece in our design is an ordered update protocol. A normal link-state protocol can still create a short-lived forwarding loop if different switches install a new route at slightly different times. We saw that happen in two practice failure schedules, so we added a small agreement barrier and update routes from the destination outward. This keeps both the final routes and the transition between routes loop-free in our tests.

## What a switch knows

At startup, a switch knows only its own switch ID and its local port numbers. It is not told what is on the other end of a port, which port faces a customer, the network topology, or when a link fails.

The control plane runs when a packet is punted or a timer fires. We use those two events to discover everything else:

- A reserved control protocol identifies our own control packets.
- Periodic `HELLO` messages reveal neighboring switches and provide liveness information.
- An ordinary packet arriving on a port that never sends a HELLO identifies a possible local customer.
- A route miss exposes the packet's source and destination addresses, but not the customer's prefix length. For that reason, we install exact `/32` routes for observed customer addresses instead of guessing a prefix.

## Data plane

The data plane has two exact-match tables:

1. A control table matches IP protocol 253 and punts the packet to the Rust controller.
2. A route table matches the customer's destination address and sets the egress port.

If neither table produces an egress port, the simulator punts the packet with `NoRoute`. Route entries use stable IDs. When a next hop changes, we delete the old entry before installing the replacement because simulator installs append rather than replace.

This keeps the fast path small: customer packets normally require one destination lookup and never enter the controller.

## Neighbor and customer discovery

Every switch sends HELLOs every 25 ms. A HELLO contains the sender's switch ID and an increasing sequence number. A port is considered a live switch link after a fresh HELLO arrives. Equal or older HELLOs are ignored so a delayed duplicate cannot make a failed link appear alive.

Customer discovery is deliberately slower. If ordinary traffic arrives on an unknown port, we record the source as a candidate. The port becomes a customer port only after 50 ms without a valid HELLO. This avoids mistaking early transit traffic for a local customer. If a HELLO later appears, switch-neighbor evidence wins and the tentative customer information is removed.

## Sharing topology information

Each switch originates a link-state advertisement, or LSA. An LSA is a complete snapshot containing:

- the origin switch ID;
- an increasing sequence number;
- the origin's currently live switch neighbors; and
- the customer addresses attached to that origin.

A receiver keeps only the newest sequence number from each origin and floods a new LSA to its other live neighbors. Complete snapshots make removal simple: if a newer LSA omits a link or customer, that fact is gone. We also send one rotating LSDB record periodically so a lost flood is eventually repaired.

For routing, a physical link is usable only when both endpoints advertise each other. This mutual-adjacency rule prevents a one-sided discovery or stale LSA from creating a route over a link that is not actually ready.

## Computing routes

All A1 links have equal cost, so breadth-first search is enough. For each customer, we compute the distance from every switch to the customer's switch. A switch chooses a directly connected neighbor whose distance is exactly one smaller than its own. Equal-length choices are broken by neighbor ID and then port ID, which makes every switch's decision deterministic.

In a settled topology, the distance decreases at every hop:

```text
3 -> 2 -> 1 -> 0 -> customer
```

That gives a simple loop-free invariant: a packet cannot keep moving forever while its remaining distance strictly decreases.

## Avoiding loops while routes change

The harder problem is the transition between two valid topologies. If switches update independently, switch A might use its new route through B while B still uses an old route through A. Both routes were computed correctly, but the mixed state briefly forms an A-B loop.

We prevent this in two steps.

First, every switch computes a 128-bit ID for its complete ordered link-state database. The lowest-ID switch in the current connected component acts as leader. Every member sends `READY` for its view, and the leader starts an update only after every member reports the same view.

Second, the leader runs distance phases:

- Phase 0 removes unreachable routes and installs local customer routes.
- Phase 1 updates switches one hop from the destination.
- Phase 2 updates switches two hops away.
- Later phases continue outward.

A switch acknowledges a phase only after it has submitted all of that phase's local table changes. The leader waits for every acknowledgement before starting the next phase. Therefore, when a farther switch redirects traffic, the closer next hop is already prepared.

Control messages carry the exact view, leader, generation, and phase. A newer LSA cancels the old update, and delayed messages from an older generation are rejected. READY messages and incomplete phases retry every 25 ms, so one lost control packet does not cause a permanent wait.

## Link failure and recovery

Failures are silent in A1. If no fresh HELLO arrives for 100 ms, the neighbor is marked down. Routes using that local port are removed immediately, a newer LSA is flooded, and the switches coordinate alternate routes for the new view.

We continue sending HELLOs on a down port. When the link recovers, a fresh HELLO crosses it, both endpoints advertise the adjacency again, and the link becomes usable after both endpoint claims are known. The same ordered update process installs any recovered shorter paths.

We intentionally prefer a brief missing route over forwarding through an unsafe mixed state. In the supplied failure tests these withdrawals were short, every destination became reachable again, and recovery stayed well below the 1,000 ms limit.

## Tradeoffs and results

Link-state routing stores more topology information than a distance-vector design, and the READY/acknowledgement protocol adds control traffic and state. We chose it because A1 networks are small, shortest paths are easy to reason about, and explicit coordination fixed a real transient-loop failure rather than relying on a timing delay.

The final program passed all five supplied Part 1 worlds with 100% scored delivery. It also passed all 15 Part 2 schedules: every ordered application pair was reachable, all 180 link events recovered, no packet revisited a switch or died from TTL, and the worst recovery was 145 ms. Compared with the earlier independently updating version, total scored loss fell by about 23% even with the additional coordination.

The main limitation is that customer prefix length is not visible to the program, so the implementation routes observed application addresses rather than unseen addresses within a larger prefix. The 128-bit view ID also has a theoretical hash-collision risk, although it is negligible at A1 scale.
