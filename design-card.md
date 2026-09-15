# NetworkArena A1 Design Card

## Overview

Our program uses a link-state routing design. Each switch discovers its local neighbors and customer address, floods that information through the network, and computes shortest paths from the resulting topology. The data plane stays simple: one table punts our control protocol, and a second exact-match table forwards customer packets by destination address.

We chose link-state routing because A1 networks are small and use equal-cost links. A complete topology makes shortest-path routing deterministic and failure handling easy to reason about.

## Discovery and topology

Switches send periodic `HELLO` messages to identify neighboring switches and check liveness. Ordinary traffic on a port that never receives a HELLO identifies a possible local customer. We briefly delay that classification so startup timing does not cause a switch link to be mistaken for a customer port.

Each switch advertises a complete snapshot of its live neighbors and local customer addresses. Sequence numbers reject stale information. New advertisements are flooded immediately, while a periodic refresh repairs a missed flood.

We treat a link as usable only when both endpoints advertise each other. This mutual-adjacency rule avoids routing over a link that is only partially discovered or has stale state on one side.

## Forwarding

For each customer address, the controller runs breadth-first search over the mutual-link graph. It chooses a directly connected neighbor whose distance to the destination is one less than the current switch's distance. Equal-length paths are broken deterministically by neighbor ID and then port ID.

Distance strictly decreases at every hop, so a settled route cannot contain a loop. Customer routes use the exact addresses visible to the control plane.

## Failure and recovery

Link failures are silent. After 100 ms without a fresh HELLO, a switch marks the link down, removes routes using that port, and advertises the change. The network then computes alternate paths.

HELLOs continue on down ports. When a link recovers, fresh HELLOs cross again and both endpoints advertise the restored adjacency.

## Updating routes without transient loops

The first version installed new shortest paths independently at each switch. Although the final routes were loop-free, two switches could briefly use routes from different topology versions and point toward each other.

To prevent that, switches compute an ID for their complete routing view. The lowest-ID switch in the connected component acts as leader and waits until every member reports the same view before starting an update.

Routes are then installed in distance phases, working outward from each destination:

- Phase 0 removes unreachable routes and installs local customer routes.
- Phase 1 updates switches one hop away.
- Later phases continue with increasing distance.

Each switch acknowledges a phase only after submitting all of its local table changes. The leader begins the next phase only after every member acknowledges. This ensures that a switch's new next hop is ready before traffic is redirected toward it.

A newer topology advertisement cancels an update already in progress. Versioned control messages prevent a delayed update from overwriting newer forwarding state, and incomplete updates are retried.

## Tradeoffs

The agreement barrier and acknowledgements add control traffic and state, but they remove a real transient-loop case rather than relying on a timing delay. During failure handling, we prefer a short missing route over forwarding through an unsafe mixed state.

This design passed all supplied Part 1 and Part 2 runs, maintained reachability, recovered within the required limit, and produced no observed forwarding loops or TTL deaths.
