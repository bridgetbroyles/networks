//! Assignment 1 switch: bounded link-state discovery and exact host routing.

use std::collections::{BTreeMap, BTreeSet};

use switch_program_sdk::*;

mod protocol;
mod routing;

use protocol::{ControlMessage, Hello, Lsa, PhaseAck, Ready, UpdatePhase, ViewId};
use routing::DesiredRoute;

const T_CONTROL: u32 = 1;
const T_ROUTE: u32 = 2;

const CONTROL_PROTO: u8 = 253;
const CONTROL_REASON: u32 = 0x4e41_3101;
const CONTROL_DST: u32 = 0xe000_00fd;
const CONTROL_SOURCE_BASE: u32 = 0xa9fe_0000;

const HELLO_INTERVAL_NS: u64 = 25_000_000;
const PORT_CLASSIFY_NS: u64 = 50_000_000;
const NEIGHBOR_DEAD_NS: u64 = 100_000_000;
const PHASE_RETRY_NS: u64 = HELLO_INTERVAL_NS;

const ROUTE_TABLE_CAPACITY: u32 = 256;

/// Conservative postcard-size estimate. The real host limit is 4096 bytes;
/// leaving headroom also covers enum/vector framing we do not calculate here.
const ACTION_BUDGET_BYTES: usize = 3_200;
const CONTROL_ACTION_OVERHEAD: usize = 48;
const INSTALL_ACTION_ESTIMATE: usize = 64;
const DELETE_ACTION_ESTIMATE: usize = 32;
const TIMER_ACTION_ESTIMATE: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PortRole {
    Unknown,
    Switch { neighbor_id: u32, live: bool },
    Customer,
}

#[derive(Debug, Clone)]
struct PortState {
    role: PortRole,
    last_hello_ns: Option<u64>,
    last_hello_sequence: Option<u64>,
    first_data_ns: Option<u64>,
    candidate_customers: BTreeSet<u32>,
}

impl PortState {
    fn unknown() -> Self {
        Self {
            role: PortRole::Unknown,
            last_hello_ns: None,
            last_hello_sequence: None,
            first_data_ns: None,
            candidate_customers: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InstalledRoute {
    entry_id: u64,
    port: u16,
    next_hop: Option<u32>,
}

#[derive(Debug, Clone)]
struct ActiveUpdate {
    leader: u32,
    generation: u64,
    view: ViewId,
    max_phase: u32,
    completed_phase: Option<u32>,
    desired: BTreeMap<u32, DesiredRoute>,
}

#[derive(Debug, Clone)]
struct LeaderUpdate {
    generation: u64,
    view: ViewId,
    members: BTreeSet<u32>,
    phase: u32,
    max_phase: u32,
    attempt: u64,
    acknowledged: BTreeSet<u32>,
    last_send_ns: u64,
}

struct ActionBatch {
    actions: Vec<Action>,
    estimated_bytes: usize,
    limit: usize,
}

impl ActionBatch {
    fn new(limit: usize) -> Self {
        Self {
            actions: Vec::new(),
            estimated_bytes: 0,
            limit,
        }
    }

    fn can_fit(&self, estimated_bytes: usize) -> bool {
        self.estimated_bytes.saturating_add(estimated_bytes) <= self.limit
    }

    fn try_push(&mut self, action: Action, estimated_bytes: usize) -> bool {
        if !self.can_fit(estimated_bytes) {
            return false;
        }
        self.actions.push(action);
        self.estimated_bytes += estimated_bytes;
        true
    }

    fn has_work(&self) -> bool {
        !self.actions.is_empty()
    }

    fn into_actions(self) -> Vec<Action> {
        self.actions
    }

    fn into_actions_with_timer(mut self) -> Vec<Action> {
        // Timer space was reserved by constructing this batch with the smaller
        // limit in `on_timer`.
        self.actions
            .push(actions::schedule_timer(HELLO_INTERVAL_NS));
        self.actions
    }
}

pub struct A1Switch {
    switch_id: u32,
    ports: BTreeMap<u16, PortState>,
    hello_sequence: u64,
    local_lsa_sequence: u64,
    lsdb: BTreeMap<u32, Lsa>,
    local_customers: BTreeMap<u32, u16>,
    installed_routes: BTreeMap<u32, InstalledRoute>,
    routing_dirty: bool,
    sync_cursor: usize,
    ready_sequence: u64,
    update_generation: u64,
    announced_view: Option<ViewId>,
    last_ready_send_ns: u64,
    ready_by_sender: BTreeMap<u32, Ready>,
    active_update: Option<ActiveUpdate>,
    leader_update: Option<LeaderUpdate>,
    latest_generation: BTreeMap<u32, u64>,
    seen_phase_attempt: BTreeMap<(u32, u64, u32), u64>,
}

impl A1Switch {
    fn new(switch_id: u32, local_ports: Vec<u16>) -> Self {
        let ports = local_ports
            .into_iter()
            .map(|port| (port, PortState::unknown()))
            .collect();
        let mut lsdb = BTreeMap::new();
        lsdb.insert(switch_id, Lsa::new(switch_id, 0, Vec::new(), Vec::new()));
        Self {
            switch_id,
            ports,
            hello_sequence: 0,
            local_lsa_sequence: 0,
            lsdb,
            local_customers: BTreeMap::new(),
            installed_routes: BTreeMap::new(),
            routing_dirty: true,
            sync_cursor: 0,
            ready_sequence: 0,
            update_generation: 0,
            announced_view: None,
            last_ready_send_ns: 0,
            ready_by_sender: BTreeMap::new(),
            active_update: None,
            leader_update: None,
            latest_generation: BTreeMap::new(),
            seen_phase_attempt: BTreeMap::new(),
        }
    }

    fn control_source(&self) -> u32 {
        CONTROL_SOURCE_BASE | (self.switch_id & 0xffff)
    }

    fn send_control(&self, port: u16, payload: Vec<u8>, batch: &mut ActionBatch) -> bool {
        let estimate = CONTROL_ACTION_OVERHEAD.saturating_add(payload.len());
        batch.try_push(
            actions::inject_packet(
                port,
                self.control_source(),
                CONTROL_DST,
                CONTROL_PROTO,
                1,
                payload,
            ),
            estimate,
        )
    }

    fn live_neighbor_ports(&self) -> Vec<(u16, u32)> {
        self.ports
            .iter()
            .filter_map(|(&port, state)| match state.role {
                PortRole::Switch {
                    neighbor_id,
                    live: true,
                } => Some((port, neighbor_id)),
                _ => None,
            })
            .collect()
    }

    fn direct_neighbors(&self) -> Vec<(u32, u16)> {
        let mut neighbors: Vec<(u32, u16)> = self
            .live_neighbor_ports()
            .into_iter()
            .map(|(port, neighbor)| (neighbor, port))
            .collect();
        neighbors.sort_unstable();
        neighbors
    }

    fn flood_lsa(&self, lsa: &Lsa, except_port: Option<u16>, batch: &mut ActionBatch) {
        let Some(payload) = protocol::encode_lsa(lsa) else {
            return;
        };
        for (port, _) in self.live_neighbor_ports() {
            if Some(port) != except_port {
                let _ = self.send_control(port, payload.clone(), batch);
            }
        }
    }

    fn sync_neighbor(&self, port: u16, batch: &mut ActionBatch) {
        for lsa in self.lsdb.values() {
            let Some(payload) = protocol::encode_lsa(lsa) else {
                continue;
            };
            if !self.send_control(port, payload, batch) {
                break;
            }
        }
    }

    fn mark_routing_change(&mut self, _now_ns: u64) {
        self.routing_dirty = true;
        self.announced_view = None;
        self.active_update = None;
        self.leader_update = None;
        self.seen_phase_attempt.clear();
    }

    /// Publish a new complete local snapshot only when local facts changed.
    fn refresh_local_lsa(&mut self, now_ns: u64, batch: &mut ActionBatch) -> bool {
        let mut neighbors: Vec<u32> = self
            .live_neighbor_ports()
            .into_iter()
            .map(|(_, neighbor)| neighbor)
            .collect();
        neighbors.sort_unstable();
        neighbors.dedup();
        let customers: Vec<u32> = self.local_customers.keys().copied().collect();

        let unchanged = self.lsdb.get(&self.switch_id).is_some_and(|current| {
            current.neighbors == neighbors && current.customers == customers
        });
        if unchanged {
            return false;
        }

        self.local_lsa_sequence = self.local_lsa_sequence.saturating_add(1);
        let lsa = Lsa::new(
            self.switch_id,
            self.local_lsa_sequence,
            neighbors,
            customers,
        );
        self.lsdb.insert(self.switch_id, lsa.clone());
        self.mark_routing_change(now_ns);
        self.flood_lsa(&lsa, None, batch);
        true
    }

    fn remove_local_customers_on_port(&mut self, port: u16) -> bool {
        let removed: Vec<u32> = self
            .local_customers
            .iter()
            .filter_map(|(&customer, &customer_port)| (customer_port == port).then_some(customer))
            .collect();
        for customer in &removed {
            self.local_customers.remove(customer);
        }
        !removed.is_empty()
    }

    fn withdraw_installed_routes_on_port(&mut self, port: u16, batch: &mut ActionBatch) {
        let affected: Vec<u32> = self
            .installed_routes
            .iter()
            .filter_map(|(&customer, route)| (route.port == port).then_some(customer))
            .collect();
        for customer in affected {
            let Some(route) = self.installed_routes.get(&customer).copied() else {
                continue;
            };
            if batch.try_push(
                actions::delete_entry(T_ROUTE, route.entry_id),
                DELETE_ACTION_ESTIMATE,
            ) {
                self.installed_routes.remove(&customer);
            }
        }
    }

    fn handle_hello(
        &mut self,
        now_ns: u64,
        ingress_port: u16,
        hello: Hello,
        batch: &mut ActionBatch,
    ) {
        if hello.sender == self.switch_id || !self.ports.contains_key(&ingress_port) {
            return;
        }

        let mut local_changed = false;
        let mut was_customer = false;
        {
            let state = self
                .ports
                .get_mut(&ingress_port)
                .expect("port checked above");
            if let PortRole::Switch { neighbor_id, .. } = state.role {
                if neighbor_id == hello.sender
                    && state
                        .last_hello_sequence
                        .is_some_and(|last| hello.sequence <= last)
                {
                    return;
                }
            }

            match state.role {
                PortRole::Unknown => local_changed = true,
                PortRole::Customer => {
                    local_changed = true;
                    was_customer = true;
                }
                PortRole::Switch { neighbor_id, live } => {
                    if neighbor_id != hello.sender || !live {
                        local_changed = true;
                    }
                }
            }

            state.role = PortRole::Switch {
                neighbor_id: hello.sender,
                live: true,
            };
            state.last_hello_ns = Some(now_ns);
            state.last_hello_sequence = Some(hello.sequence);
            state.first_data_ns = None;
            state.candidate_customers.clear();
        }

        if was_customer {
            let removed = self.remove_local_customers_on_port(ingress_port);
            self.withdraw_installed_routes_on_port(ingress_port, batch);
            local_changed |= removed;
        }

        if local_changed {
            let _ = self.refresh_local_lsa(now_ns, batch);
            // Reply immediately and seed the recovered/new link with our DB.
            let hello_payload = protocol::encode_hello(self.switch_id, self.hello_sequence);
            let _ = self.send_control(ingress_port, hello_payload, batch);
            self.sync_neighbor(ingress_port, batch);
        }
    }

    fn handle_lsa(&mut self, now_ns: u64, ingress_port: u16, lsa: Lsa, batch: &mut ActionBatch) {
        let valid_ingress = self
            .ports
            .get(&ingress_port)
            .is_some_and(|state| matches!(state.role, PortRole::Switch { live: true, .. }));
        if !valid_ingress || lsa.origin == self.switch_id {
            return;
        }
        if self
            .lsdb
            .get(&lsa.origin)
            .is_some_and(|known| known.sequence >= lsa.sequence)
        {
            return;
        }

        self.lsdb.insert(lsa.origin, lsa.clone());
        self.mark_routing_change(now_ns);
        self.flood_lsa(&lsa, Some(ingress_port), batch);
    }

    fn handle_customer_punt(
        &mut self,
        now_ns: u64,
        ingress_port: u16,
        source: u32,
        batch: &mut ActionBatch,
    ) {
        if source == 0 {
            return;
        }
        let mut newly_confirmed = false;
        let Some(state) = self.ports.get_mut(&ingress_port) else {
            return;
        };
        match state.role {
            PortRole::Unknown => {
                state.first_data_ns.get_or_insert(now_ns);
                state.candidate_customers.insert(source);
            }
            PortRole::Customer => {
                state.candidate_customers.insert(source);
                newly_confirmed = self.local_customers.insert(source, ingress_port).is_none();
            }
            PortRole::Switch { .. } => {}
        }
        if newly_confirmed {
            let _ = self.refresh_local_lsa(now_ns, batch);
        }
    }

    fn expire_neighbors(&mut self, now_ns: u64) -> Vec<u16> {
        let mut expired = Vec::new();
        for (&port, state) in &mut self.ports {
            let should_expire = matches!(state.role, PortRole::Switch { live: true, .. })
                && state
                    .last_hello_ns
                    .is_some_and(|last| now_ns.saturating_sub(last) >= NEIGHBOR_DEAD_NS);
            if should_expire {
                if let PortRole::Switch { neighbor_id, .. } = state.role {
                    state.role = PortRole::Switch {
                        neighbor_id,
                        live: false,
                    };
                    expired.push(port);
                }
            }
        }
        expired
    }

    fn classify_customer_ports(&mut self, now_ns: u64) -> bool {
        let mut learned = Vec::new();
        for (&port, state) in &mut self.ports {
            let ready = matches!(state.role, PortRole::Unknown)
                && state
                    .first_data_ns
                    .is_some_and(|first| now_ns.saturating_sub(first) >= PORT_CLASSIFY_NS)
                && !state.candidate_customers.is_empty();
            if ready {
                state.role = PortRole::Customer;
                learned.extend(
                    state
                        .candidate_customers
                        .iter()
                        .copied()
                        .map(|customer| (customer, port)),
                );
            }
        }

        let mut changed = false;
        for (customer, port) in learned {
            if self.local_customers.insert(customer, port) != Some(port) {
                changed = true;
            }
        }
        changed
    }

    fn view_id(&self) -> ViewId {
        fn mix(hash: &mut u64, value: u64, multiplier: u64) {
            *hash ^= value.wrapping_add(0x9e37_79b9_7f4a_7c15);
            *hash = hash.wrapping_mul(multiplier);
            *hash ^= *hash >> 29;
        }

        let mut high = 0xcbf2_9ce4_8422_2325;
        let mut low = 0x6eed_0e9d_a4d9_4a4f;
        for (&origin, lsa) in &self.lsdb {
            for value in [
                0x4c53_4100_0000_0001,
                u64::from(origin),
                lsa.sequence,
                lsa.neighbors.len() as u64,
            ] {
                mix(&mut high, value, 0x0000_0100_0000_01b3);
                mix(&mut low, value.rotate_left(23), 0x9e37_79b1_85eb_ca87);
            }
            for &neighbor in &lsa.neighbors {
                let value = 0x4e00_0000_0000_0000 | u64::from(neighbor);
                mix(&mut high, value, 0x0000_0100_0000_01b3);
                mix(&mut low, value.rotate_left(23), 0x9e37_79b1_85eb_ca87);
            }
            mix(&mut high, lsa.customers.len() as u64, 0x0000_0100_0000_01b3);
            mix(
                &mut low,
                (lsa.customers.len() as u64).rotate_left(23),
                0x9e37_79b1_85eb_ca87,
            );
            for &customer in &lsa.customers {
                let value = 0x4300_0000_0000_0000 | u64::from(customer);
                mix(&mut high, value, 0x0000_0100_0000_01b3);
                mix(&mut low, value.rotate_left(23), 0x9e37_79b1_85eb_ca87);
            }
        }
        ViewId { high, low }
    }

    fn coordination_context(&self) -> (ViewId, BTreeSet<u32>, u32) {
        let members = routing::component_members(self.switch_id, &self.lsdb);
        let leader = members.iter().next().copied().unwrap_or(self.switch_id);
        (self.view_id(), members, leader)
    }

    fn send_toward(&self, target: u32, payload: Vec<u8>, batch: &mut ActionBatch) -> bool {
        let Some((_, port)) =
            routing::next_hop_to(self.switch_id, target, &self.direct_neighbors(), &self.lsdb)
        else {
            return false;
        };
        self.send_control(port, payload, batch)
    }

    fn flood_control(&self, payload: Vec<u8>, except_port: Option<u16>, batch: &mut ActionBatch) {
        for (port, _) in self.live_neighbor_ports() {
            if Some(port) != except_port {
                let _ = self.send_control(port, payload.clone(), batch);
            }
        }
    }

    fn desired_routes(&self) -> BTreeMap<u32, DesiredRoute> {
        routing::desired_routes(
            self.switch_id,
            &self.direct_neighbors(),
            &self.local_customers,
            &self.lsdb,
        )
    }

    /// Apply only the route changes whose new distance equals `phase`.
    /// Withdrawals have phase zero. The caller acknowledges the phase only
    /// when every matching action fit in the output budget.
    fn apply_route_phase(
        &mut self,
        desired: &BTreeMap<u32, DesiredRoute>,
        phase: u32,
        batch: &mut ActionBatch,
    ) -> bool {
        let keys: BTreeSet<u32> = self
            .installed_routes
            .keys()
            .chain(desired.keys())
            .copied()
            .collect();
        let mut complete = true;

        for customer in keys {
            let old = self.installed_routes.get(&customer).copied();
            let new = desired.get(&customer).copied();
            let unchanged = matches!((old, new), (Some(old), Some(new))
                if old.port == new.port && old.next_hop == new.next_hop);
            if unchanged || matches!((old, new), (None, None)) {
                continue;
            }
            let route_phase = new.map_or(0, |route| route.distance);
            if route_phase != phase {
                continue;
            }
            match (old, new) {
                (None, None) => {}
                (None, Some(route)) => {
                    let installed = InstalledRoute {
                        entry_id: u64::from(customer),
                        port: route.port,
                        next_hop: route.next_hop,
                    };
                    if batch.try_push(
                        actions::install_route(
                            T_ROUTE,
                            installed.entry_id,
                            u64::from(customer),
                            32,
                            installed.port,
                        ),
                        INSTALL_ACTION_ESTIMATE,
                    ) {
                        self.installed_routes.insert(customer, installed);
                    } else {
                        complete = false;
                    }
                }
                (Some(old_route), None) => {
                    if batch.try_push(
                        actions::delete_entry(T_ROUTE, old_route.entry_id),
                        DELETE_ACTION_ESTIMATE,
                    ) {
                        self.installed_routes.remove(&customer);
                    } else {
                        complete = false;
                    }
                }
                (Some(old_route), Some(route)) => {
                    let pair_cost = DELETE_ACTION_ESTIMATE + INSTALL_ACTION_ESTIMATE;
                    if !batch.can_fit(pair_cost) {
                        complete = false;
                        continue;
                    }
                    let _ = batch.try_push(
                        actions::delete_entry(T_ROUTE, old_route.entry_id),
                        DELETE_ACTION_ESTIMATE,
                    );
                    let replacement = InstalledRoute {
                        entry_id: old_route.entry_id,
                        port: route.port,
                        next_hop: route.next_hop,
                    };
                    let _ = batch.try_push(
                        actions::install_route(
                            T_ROUTE,
                            replacement.entry_id,
                            u64::from(customer),
                            32,
                            replacement.port,
                        ),
                        INSTALL_ACTION_ESTIMATE,
                    );
                    self.installed_routes.insert(customer, replacement);
                }
            }
        }
        complete
    }

    fn send_ready(&mut self, now_ns: u64, ready: Ready, batch: &mut ActionBatch) {
        self.last_ready_send_ns = now_ns;
        if ready.leader == self.switch_id {
            self.ready_by_sender.insert(self.switch_id, ready);
            self.maybe_start_leader_update(now_ns, batch);
        } else {
            let _ = self.send_toward(ready.leader, protocol::encode_ready(ready), batch);
        }
    }

    fn maybe_announce_ready(&mut self, now_ns: u64, batch: &mut ActionBatch) {
        if !self.routing_dirty {
            return;
        }
        let (view, _, leader) = self.coordination_context();
        let is_new = self.announced_view != Some(view);
        if is_new {
            self.ready_sequence = self.ready_sequence.saturating_add(1);
            self.announced_view = Some(view);
        }
        if is_new || now_ns.saturating_sub(self.last_ready_send_ns) >= PHASE_RETRY_NS {
            self.send_ready(
                now_ns,
                Ready {
                    sender: self.switch_id,
                    leader,
                    sequence: self.ready_sequence,
                    view,
                },
                batch,
            );
        }
    }

    fn handle_ready(&mut self, now_ns: u64, ready: Ready, batch: &mut ActionBatch) {
        let (view, members, leader) = self.coordination_context();
        if self.announced_view != Some(view)
            || ready.view != view
            || ready.leader != leader
            || !members.contains(&ready.sender)
        {
            return;
        }
        if leader != self.switch_id {
            let _ = self.send_toward(leader, protocol::encode_ready(ready), batch);
            return;
        }
        let accept = self
            .ready_by_sender
            .get(&ready.sender)
            .is_none_or(|known| ready.sequence >= known.sequence);
        if accept {
            self.ready_by_sender.insert(ready.sender, ready);
            self.maybe_start_leader_update(now_ns, batch);
        }
    }

    fn maybe_start_leader_update(&mut self, now_ns: u64, batch: &mut ActionBatch) {
        if !self.routing_dirty || self.leader_update.is_some() {
            return;
        }
        let (view, members, leader) = self.coordination_context();
        if leader != self.switch_id || self.announced_view != Some(view) {
            return;
        }
        let all_ready = members.iter().all(|member| {
            self.ready_by_sender
                .get(member)
                .is_some_and(|ready| ready.leader == leader && ready.view == view)
        });
        if !all_ready {
            return;
        }

        self.update_generation = self.update_generation.saturating_add(1);
        let max_phase = members.len().saturating_sub(1) as u32;
        let generation = self.update_generation;
        self.latest_generation.insert(leader, generation);
        self.active_update = Some(ActiveUpdate {
            leader,
            generation,
            view,
            max_phase,
            completed_phase: None,
            desired: self.desired_routes(),
        });
        self.leader_update = Some(LeaderUpdate {
            generation,
            view,
            members,
            phase: 0,
            max_phase,
            attempt: 0,
            acknowledged: BTreeSet::new(),
            last_send_ns: now_ns,
        });
        self.launch_current_phase(now_ns, batch);
    }

    fn phase_message(&self) -> Option<UpdatePhase> {
        let update = self.leader_update.as_ref()?;
        Some(UpdatePhase {
            leader: self.switch_id,
            generation: update.generation,
            view: update.view,
            phase: update.phase,
            max_phase: update.max_phase,
            attempt: update.attempt,
        })
    }

    fn apply_active_phase(&mut self, phase: UpdatePhase, batch: &mut ActionBatch) -> bool {
        let Some(active) = self.active_update.as_ref() else {
            return false;
        };
        if active.leader != phase.leader
            || active.generation != phase.generation
            || active.view != phase.view
            || active.max_phase != phase.max_phase
        {
            return false;
        }
        let expected = active
            .completed_phase
            .map_or(0, |done| done.saturating_add(1));
        if phase.phase < expected {
            return true;
        }
        if phase.phase > expected {
            return false;
        }
        let desired = active.desired.clone();
        if !self.apply_route_phase(&desired, phase.phase, batch) {
            return false;
        }
        if let Some(active) = self.active_update.as_mut() {
            active.completed_phase = Some(phase.phase);
        }
        true
    }

    fn launch_current_phase(&mut self, now_ns: u64, batch: &mut ActionBatch) {
        let Some(phase) = self.phase_message() else {
            return;
        };
        let local_complete = self.apply_active_phase(phase, batch);
        if let Some(update) = self.leader_update.as_mut() {
            update.last_send_ns = now_ns;
            if local_complete {
                update.acknowledged.insert(self.switch_id);
            }
        }
        self.flood_control(protocol::encode_update_phase(phase), None, batch);
        self.maybe_advance_phase(now_ns, batch);
    }

    fn maybe_advance_phase(&mut self, now_ns: u64, batch: &mut ActionBatch) {
        let complete = self.leader_update.as_ref().is_some_and(|update| {
            update
                .members
                .iter()
                .all(|member| update.acknowledged.contains(member))
        });
        if !complete {
            return;
        }
        let finished = self
            .leader_update
            .as_ref()
            .is_some_and(|update| update.phase >= update.max_phase);
        if finished {
            self.routing_dirty = false;
            self.leader_update = None;
            return;
        }
        if let Some(update) = self.leader_update.as_mut() {
            update.phase += 1;
            update.attempt = 0;
            update.acknowledged.clear();
        }
        self.launch_current_phase(now_ns, batch);
    }

    fn handle_update_phase(
        &mut self,
        _now_ns: u64,
        ingress_port: u16,
        phase: UpdatePhase,
        batch: &mut ActionBatch,
    ) {
        let (view, members, leader) = self.coordination_context();
        if self.announced_view != Some(view)
            || phase.view != view
            || phase.leader != leader
            || phase.max_phase != members.len().saturating_sub(1) as u32
            || phase.phase > phase.max_phase
        {
            return;
        }
        let known_generation = self.latest_generation.get(&leader).copied().unwrap_or(0);
        if phase.generation < known_generation {
            return;
        }
        if phase.generation > known_generation {
            self.latest_generation.insert(leader, phase.generation);
            self.active_update = Some(ActiveUpdate {
                leader,
                generation: phase.generation,
                view,
                max_phase: phase.max_phase,
                completed_phase: None,
                desired: self.desired_routes(),
            });
        }

        let key = (leader, phase.generation, phase.phase);
        let should_flood = self
            .seen_phase_attempt
            .get(&key)
            .is_none_or(|known| phase.attempt > *known);
        if should_flood {
            self.seen_phase_attempt.insert(key, phase.attempt);
        }

        if self.apply_active_phase(phase, batch) {
            let ack = PhaseAck {
                sender: self.switch_id,
                leader,
                generation: phase.generation,
                view,
                phase: phase.phase,
                attempt: phase.attempt,
            };
            let _ = self.send_toward(leader, protocol::encode_phase_ack(ack), batch);
            if phase.phase == phase.max_phase {
                self.routing_dirty = false;
            }
        }
        if should_flood {
            self.flood_control(
                protocol::encode_update_phase(phase),
                Some(ingress_port),
                batch,
            );
        }
    }

    fn handle_phase_ack(&mut self, now_ns: u64, ack: PhaseAck, batch: &mut ActionBatch) {
        let (view, members, leader) = self.coordination_context();
        if ack.view != view
            || ack.leader != leader
            || !members.contains(&ack.sender)
            || self.announced_view != Some(view)
        {
            return;
        }
        if leader != self.switch_id {
            let _ = self.send_toward(leader, protocol::encode_phase_ack(ack), batch);
            return;
        }
        let Some(update) = self.leader_update.as_mut() else {
            return;
        };
        if ack.generation != update.generation
            || ack.view != update.view
            || ack.phase != update.phase
        {
            return;
        }
        update.acknowledged.insert(ack.sender);
        self.maybe_advance_phase(now_ns, batch);
    }

    fn retry_coordination(&mut self, now_ns: u64, batch: &mut ActionBatch) {
        let retry_phase = self
            .leader_update
            .as_ref()
            .is_some_and(|update| now_ns.saturating_sub(update.last_send_ns) >= PHASE_RETRY_NS);
        if retry_phase {
            if let Some(update) = self.leader_update.as_mut() {
                update.attempt = update.attempt.saturating_add(1);
            }
            self.launch_current_phase(now_ns, batch);
        }
    }

    fn emit_hellos(&mut self, batch: &mut ActionBatch) {
        self.hello_sequence = self.hello_sequence.saturating_add(1);
        let payload = protocol::encode_hello(self.switch_id, self.hello_sequence);
        for (&port, state) in &self.ports {
            if !matches!(state.role, PortRole::Customer) {
                let _ = self.send_control(port, payload.clone(), batch);
            }
        }
    }

    /// Send one rotating LSDB record to every live neighbor. This repairs a
    /// missed flood without ever returning the full database per port.
    fn emit_anti_entropy(&mut self, batch: &mut ActionBatch) {
        if self.lsdb.is_empty() {
            return;
        }
        let index = self.sync_cursor % self.lsdb.len();
        self.sync_cursor = self.sync_cursor.wrapping_add(1);
        let Some(lsa) = self.lsdb.values().nth(index).cloned() else {
            return;
        };
        self.flood_lsa(&lsa, None, batch);
    }
}

impl SwitchProgram for A1Switch {
    fn init(switch_id: u32, local_ports: Vec<u16>) -> (Self, ProgramSetup) {
        let mut setup = ProgramSetup::new();
        setup.declare_table(T_CONTROL, MatchKind::Exact, 4);
        setup.declare_table(T_ROUTE, MatchKind::Exact, ROUTE_TABLE_CAPACITY);
        setup.add_entry(
            T_CONTROL,
            TableEntry {
                id: wire::EntryIdW(1),
                key: u64::from(CONTROL_PROTO),
                prefix_len: 8,
                priority: 0,
                action: TableAction::Punt {
                    reason: PuntReason::Custom(CONTROL_REASON),
                },
            },
        );
        setup.set_program(
            text::parse_tiny_program(
                "
                stage alu=4 mem=1
                    load    r0, ip_proto
                    table   t1, r0 -> m0
                stage alu=4 mem=1
                    load    r0, ip_dst
                    table   t2, r0 -> m1
                ",
            )
            .expect("static TinyVM program must parse"),
        );
        (Self::new(switch_id, local_ports), setup)
    }

    fn on_punt(&mut self, ev: PuntEvent) -> Vec<Action> {
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        if ev.ip_proto == CONTROL_PROTO && matches!(ev.reason, PuntReason::Custom(CONTROL_REASON)) {
            if let Some(message) = protocol::decode(&ev.payload) {
                match message {
                    ControlMessage::Hello(hello) => {
                        self.handle_hello(ev.now_ns, ev.ingress_port, hello, &mut batch)
                    }
                    ControlMessage::Lsa(lsa) => {
                        self.handle_lsa(ev.now_ns, ev.ingress_port, lsa, &mut batch)
                    }
                    ControlMessage::Ready(ready) => self.handle_ready(ev.now_ns, ready, &mut batch),
                    ControlMessage::UpdatePhase(phase) => {
                        self.handle_update_phase(ev.now_ns, ev.ingress_port, phase, &mut batch)
                    }
                    ControlMessage::PhaseAck(ack) => {
                        self.handle_phase_ack(ev.now_ns, ack, &mut batch)
                    }
                }
            }
        } else if matches!(ev.reason, PuntReason::NoRoute) {
            self.handle_customer_punt(ev.now_ns, ev.ingress_port, ev.ip_src, &mut batch);
        }
        // The view-equality barrier makes a separate quiet-time hold
        // unnecessary. Advertising each newly learned view immediately avoids
        // adding a full heartbeat of latency for the final LSA.
        self.maybe_announce_ready(ev.now_ns, &mut batch);
        batch.into_actions()
    }

    fn on_timer(&mut self, ev: TimerEvent) -> Vec<Action> {
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES - TIMER_ACTION_ESTIMATE);

        let expired_ports = self.expire_neighbors(ev.now_ns);
        for port in &expired_ports {
            self.withdraw_installed_routes_on_port(*port, &mut batch);
        }
        let customers_changed = self.classify_customer_ports(ev.now_ns);
        if !expired_ports.is_empty() || customers_changed {
            let _ = self.refresh_local_lsa(ev.now_ns, &mut batch);
        }

        self.maybe_announce_ready(ev.now_ns, &mut batch);
        self.retry_coordination(ev.now_ns, &mut batch);
        let had_state_work = batch.has_work();
        self.emit_hellos(&mut batch);
        if !had_state_work {
            self.emit_anti_entropy(&mut batch);
        }

        batch.into_actions_with_timer()
    }
}

switch_program!(A1Switch);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_declares_two_small_tables() {
        let (_program, setup) = A1Switch::init(5, vec![0, 1, 100]);
        let wire = setup.into_wire();
        assert_eq!(wire.tables.len(), 2);
        assert_eq!(wire.program.stages.len(), 2);
        assert_eq!(wire.tables[1].max_entries, ROUTE_TABLE_CAPACITY);
    }

    #[test]
    fn customer_waits_for_quiet_classification_period() {
        let mut program = A1Switch::new(1, vec![7]);
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        program.handle_customer_punt(10, 7, 0x0a00_0101, &mut batch);
        assert!(!program.classify_customer_ports(10 + PORT_CLASSIFY_NS - 1));
        assert!(program.classify_customer_ports(10 + PORT_CLASSIFY_NS));
        assert_eq!(program.local_customers.get(&0x0a00_0101), Some(&7));
    }

    #[test]
    fn stale_or_duplicate_hello_cannot_refresh_liveness() {
        let mut program = A1Switch::new(1, vec![3]);
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        program.handle_hello(
            100,
            3,
            Hello {
                sender: 2,
                sequence: 10,
            },
            &mut batch,
        );
        program.handle_hello(
            150,
            3,
            Hello {
                sender: 2,
                sequence: 10,
            },
            &mut batch,
        );
        program.handle_hello(
            200,
            3,
            Hello {
                sender: 2,
                sequence: 9,
            },
            &mut batch,
        );
        assert_eq!(program.ports[&3].last_hello_ns, Some(100));
    }

    #[test]
    fn timeout_withdraws_neighbor_and_fresh_hello_recovers_it() {
        let mut program = A1Switch::new(1, vec![3]);
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        program.handle_hello(
            0,
            3,
            Hello {
                sender: 2,
                sequence: 1,
            },
            &mut batch,
        );
        assert_eq!(program.lsdb[&1].neighbors, vec![2]);
        assert!(program.expire_neighbors(NEIGHBOR_DEAD_NS - 1).is_empty());

        assert_eq!(program.expire_neighbors(NEIGHBOR_DEAD_NS), vec![3]);
        let _ = program.refresh_local_lsa(NEIGHBOR_DEAD_NS, &mut batch);
        assert!(program.lsdb[&1].neighbors.is_empty());

        program.handle_hello(
            NEIGHBOR_DEAD_NS + 1,
            3,
            Hello {
                sender: 2,
                sequence: 2,
            },
            &mut batch,
        );
        assert_eq!(program.lsdb[&1].neighbors, vec![2]);
        assert!(matches!(
            program.ports[&3].role,
            PortRole::Switch {
                neighbor_id: 2,
                live: true
            }
        ));
    }

    #[test]
    fn stale_and_duplicate_lsas_cannot_replace_newer_state() {
        let mut program = A1Switch::new(1, vec![3]);
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        program.handle_hello(
            0,
            3,
            Hello {
                sender: 2,
                sequence: 1,
            },
            &mut batch,
        );

        let original_customer = 0x0a00_0901;
        program.handle_lsa(
            1,
            3,
            Lsa::new(9, 7, vec![2], vec![original_customer]),
            &mut batch,
        );
        program.handle_lsa(2, 3, Lsa::new(9, 7, vec![], vec![0x0a00_0902]), &mut batch);
        program.handle_lsa(3, 3, Lsa::new(9, 6, vec![], vec![0x0a00_0903]), &mut batch);
        assert_eq!(program.lsdb[&9].sequence, 7);
        assert_eq!(program.lsdb[&9].customers, vec![original_customer]);

        program.handle_lsa(4, 3, Lsa::new(9, 8, vec![], vec![]), &mut batch);
        assert_eq!(program.lsdb[&9].sequence, 8);
        assert!(program.lsdb[&9].customers.is_empty());
    }

    #[test]
    fn route_diff_does_not_reinstall_unchanged_route() {
        let mut program = A1Switch::new(1, vec![7]);
        let desired = BTreeMap::from([(
            0x0a00_0101,
            DesiredRoute {
                port: 7,
                next_hop: None,
                distance: 0,
            },
        )]);
        let mut first = ActionBatch::new(ACTION_BUDGET_BYTES);
        assert!(program.apply_route_phase(&desired, 0, &mut first));
        assert_eq!(first.actions.len(), 1);

        let mut second = ActionBatch::new(ACTION_BUDGET_BYTES);
        assert!(program.apply_route_phase(&desired, 0, &mut second));
        assert!(second.actions.is_empty());
    }

    #[test]
    fn changed_route_is_deleted_before_its_replacement_is_installed() {
        let customer = 0x0a00_0101;
        let mut program = A1Switch::new(1, vec![7, 8]);
        let mut first = ActionBatch::new(ACTION_BUDGET_BYTES);
        assert!(program.apply_route_phase(
            &BTreeMap::from([(
                customer,
                DesiredRoute {
                    port: 7,
                    next_hop: Some(2),
                    distance: 1,
                },
            )]),
            1,
            &mut first,
        ));

        let mut changed = ActionBatch::new(ACTION_BUDGET_BYTES);
        assert!(program.apply_route_phase(
            &BTreeMap::from([(
                customer,
                DesiredRoute {
                    port: 8,
                    next_hop: Some(3),
                    distance: 1,
                },
            )]),
            1,
            &mut changed,
        ));
        assert!(matches!(
            changed.actions.as_slice(),
            [
                Action::DeleteTableEntry { .. },
                Action::InstallTableEntry { .. }
            ]
        ));
        assert_eq!(program.installed_routes[&customer].port, 8);
    }

    #[test]
    fn route_phase_installs_only_its_distance() {
        let mut program = A1Switch::new(1, vec![7, 8]);
        let near = 0x0a00_0101;
        let far = 0x0a00_0201;
        let desired = BTreeMap::from([
            (
                near,
                DesiredRoute {
                    port: 7,
                    next_hop: Some(2),
                    distance: 1,
                },
            ),
            (
                far,
                DesiredRoute {
                    port: 8,
                    next_hop: Some(3),
                    distance: 2,
                },
            ),
        ]);

        let mut phase_one = ActionBatch::new(ACTION_BUDGET_BYTES);
        assert!(program.apply_route_phase(&desired, 1, &mut phase_one));
        assert!(program.installed_routes.contains_key(&near));
        assert!(!program.installed_routes.contains_key(&far));

        let mut phase_two = ActionBatch::new(ACTION_BUDGET_BYTES);
        assert!(program.apply_route_phase(&desired, 2, &mut phase_two));
        assert!(program.installed_routes.contains_key(&far));
    }

    #[test]
    fn leader_waits_for_every_ready_then_advances_on_every_ack() {
        let customer = 0x0a00_0201;
        let mut program = A1Switch::new(1, vec![7]);
        program.ports.get_mut(&7).unwrap().role = PortRole::Switch {
            neighbor_id: 2,
            live: true,
        };
        program.lsdb.insert(1, Lsa::new(1, 1, vec![2], vec![]));
        program
            .lsdb
            .insert(2, Lsa::new(2, 1, vec![1], vec![customer]));
        let (view, _, leader) = program.coordination_context();
        assert_eq!(leader, 1);
        program.announced_view = Some(view);
        program.ready_by_sender.insert(
            1,
            Ready {
                sender: 1,
                leader,
                sequence: 1,
                view,
            },
        );

        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        program.maybe_start_leader_update(10, &mut batch);
        assert!(program.leader_update.is_none());

        program.handle_ready(
            11,
            Ready {
                sender: 2,
                leader,
                sequence: 1,
                view,
            },
            &mut batch,
        );
        assert_eq!(program.leader_update.as_ref().map(|u| u.phase), Some(0));
        assert!(!program.installed_routes.contains_key(&customer));

        let generation = program.leader_update.as_ref().unwrap().generation;
        program.handle_phase_ack(
            12,
            PhaseAck {
                sender: 2,
                leader,
                generation,
                view,
                phase: 0,
                attempt: 0,
            },
            &mut batch,
        );
        assert_eq!(program.leader_update.as_ref().map(|u| u.phase), Some(1));
        assert_eq!(program.installed_routes[&customer].next_hop, Some(2));

        let actions_before_retry = batch.actions.len();
        program.retry_coordination(12 + PHASE_RETRY_NS, &mut batch);
        assert_eq!(program.leader_update.as_ref().map(|u| u.attempt), Some(1));
        assert!(batch.actions.len() > actions_before_retry);
    }

    #[test]
    fn newer_lsa_cancels_an_in_progress_update() {
        let mut program = A1Switch::new(4, vec![]);
        let view = program.view_id();
        program.active_update = Some(ActiveUpdate {
            leader: 4,
            generation: 1,
            view,
            max_phase: 0,
            completed_phase: None,
            desired: BTreeMap::new(),
        });
        program.leader_update = Some(LeaderUpdate {
            generation: 1,
            view,
            members: BTreeSet::from([4]),
            phase: 0,
            max_phase: 0,
            attempt: 0,
            acknowledged: BTreeSet::new(),
            last_send_ns: 10,
        });

        program.mark_routing_change(20);
        assert!(program.routing_dirty);
        assert!(program.active_update.is_none());
        assert!(program.leader_update.is_none());
        assert!(program.announced_view.is_none());
    }

    #[test]
    fn stale_phase_cannot_replace_a_newer_generation() {
        let mut program = A1Switch::new(4, vec![]);
        let view = program.view_id();
        program.announced_view = Some(view);
        program.latest_generation.insert(4, 2);
        program.active_update = Some(ActiveUpdate {
            leader: 4,
            generation: 2,
            view,
            max_phase: 0,
            completed_phase: Some(0),
            desired: BTreeMap::new(),
        });
        let mut batch = ActionBatch::new(ACTION_BUDGET_BYTES);
        program.handle_update_phase(
            100,
            0,
            UpdatePhase {
                leader: 4,
                generation: 1,
                view,
                phase: 0,
                max_phase: 0,
                attempt: 99,
            },
            &mut batch,
        );
        assert_eq!(program.active_update.as_ref().unwrap().generation, 2);
        assert!(batch.actions.is_empty());
    }
}
