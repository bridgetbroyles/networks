//! Pure graph and route computation for the A1 controller.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::protocol::Lsa;

pub type Graph = BTreeMap<u32, BTreeSet<u32>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesiredRoute {
    pub port: u16,
    /// `None` means the destination is attached to this switch.
    pub next_hop: Option<u32>,
}

/// Construct an undirected graph containing an edge only when both latest
/// endpoint advertisements claim it.
pub fn mutual_graph(lsdb: &BTreeMap<u32, Lsa>) -> Graph {
    let mut graph: Graph = lsdb
        .keys()
        .copied()
        .map(|node| (node, BTreeSet::new()))
        .collect();

    for (&origin, lsa) in lsdb {
        for &neighbor in &lsa.neighbors {
            let mutual = lsdb
                .get(&neighbor)
                .is_some_and(|other| other.neighbors.binary_search(&origin).is_ok());
            if mutual {
                graph.entry(origin).or_default().insert(neighbor);
                graph.entry(neighbor).or_default().insert(origin);
            }
        }
    }
    graph
}

pub fn distances_from(origin: u32, graph: &Graph) -> BTreeMap<u32, u32> {
    let mut distances = BTreeMap::new();
    let mut queue = VecDeque::new();
    distances.insert(origin, 0);
    queue.push_back(origin);
    while let Some(node) = queue.pop_front() {
        let distance = distances[&node];
        for &neighbor in graph.get(&node).into_iter().flatten() {
            if let std::collections::btree_map::Entry::Vacant(entry) = distances.entry(neighbor) {
                entry.insert(distance + 1);
                queue.push_back(neighbor);
            }
        }
    }
    distances
}

/// Lowest origin ID wins an impossible/conflicting customer claim so that all
/// switches with the same LSDB make the same choice.
fn customer_origins(lsdb: &BTreeMap<u32, Lsa>) -> BTreeMap<u32, u32> {
    let mut origins = BTreeMap::new();
    for (&origin, lsa) in lsdb {
        for &customer in &lsa.customers {
            origins.entry(customer).or_insert(origin);
        }
    }
    origins
}

pub fn desired_routes(
    self_id: u32,
    direct_neighbors: &[(u32, u16)],
    local_customers: &BTreeMap<u32, u16>,
    lsdb: &BTreeMap<u32, Lsa>,
) -> BTreeMap<u32, DesiredRoute> {
    let graph = mutual_graph(lsdb);
    let origins = customer_origins(lsdb);
    let mut routes = BTreeMap::new();

    for (customer, origin) in origins {
        if origin == self_id {
            if let Some(&port) = local_customers.get(&customer) {
                routes.insert(
                    customer,
                    DesiredRoute {
                        port,
                        next_hop: None,
                    },
                );
            }
            continue;
        }

        let distances = distances_from(origin, &graph);
        let Some(&my_distance) = distances.get(&self_id) else {
            continue;
        };
        if my_distance == 0 {
            continue;
        }

        let next = direct_neighbors
            .iter()
            .filter(|(neighbor, _)| {
                graph
                    .get(&self_id)
                    .is_some_and(|adjacent| adjacent.contains(neighbor))
                    && distances.get(neighbor).copied() == Some(my_distance - 1)
            })
            .min_by_key(|(neighbor, port)| (*neighbor, *port));

        if let Some(&(neighbor, port)) = next {
            routes.insert(
                customer,
                DesiredRoute {
                    port,
                    next_hop: Some(neighbor),
                },
            );
        }
    }
    routes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lsa(origin: u32, neighbors: &[u32], customers: &[u32]) -> Lsa {
        Lsa::new(origin, 1, neighbors.to_vec(), customers.to_vec())
    }

    #[test]
    fn graph_requires_both_endpoint_claims() {
        let mut db = BTreeMap::new();
        db.insert(1, lsa(1, &[2], &[]));
        db.insert(2, lsa(2, &[], &[]));
        assert!(!mutual_graph(&db)[&1].contains(&2));
        db.insert(2, lsa(2, &[1], &[]));
        assert!(mutual_graph(&db)[&1].contains(&2));
    }

    #[test]
    fn line_routes_reduce_distance() {
        let customer = 0x0a00_0301;
        let mut db = BTreeMap::new();
        db.insert(10, lsa(10, &[20], &[]));
        db.insert(20, lsa(20, &[10, 30], &[]));
        db.insert(30, lsa(30, &[20], &[customer]));
        let local = BTreeMap::new();

        let r10 = desired_routes(10, &[(20, 7)], &local, &db);
        let r20 = desired_routes(20, &[(10, 1), (30, 2)], &local, &db);
        assert_eq!(r10[&customer].next_hop, Some(20));
        assert_eq!(r10[&customer].port, 7);
        assert_eq!(r20[&customer].next_hop, Some(30));
    }

    #[test]
    fn equal_paths_choose_lowest_neighbor_then_port() {
        let customer = 0x0a00_0401;
        let mut db = BTreeMap::new();
        db.insert(1, lsa(1, &[2, 3], &[]));
        db.insert(2, lsa(2, &[1, 4], &[]));
        db.insert(3, lsa(3, &[1, 4], &[]));
        db.insert(4, lsa(4, &[2, 3], &[customer]));
        let routes = desired_routes(1, &[(3, 9), (2, 8)], &BTreeMap::new(), &db);
        assert_eq!(routes[&customer].next_hop, Some(2));
        assert_eq!(routes[&customer].port, 8);
    }

    #[test]
    fn local_customer_uses_local_port() {
        let customer = 0x0a00_0101;
        let mut db = BTreeMap::new();
        db.insert(7, lsa(7, &[], &[customer]));
        let local = BTreeMap::from([(customer, 100)]);
        let routes = desired_routes(7, &[], &local, &db);
        assert_eq!(
            routes[&customer],
            DesiredRoute {
                port: 100,
                next_hop: None,
            }
        );
    }
}
