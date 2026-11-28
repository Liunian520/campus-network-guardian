use crate::diagnostics::{Action, Auth, Carrier, Diagnostics, External, Kind, Link, Role, Update};
use campus_network_guardian::{
    network::{self, ConnectionState, NetworkConnection, NetworkIdentity},
    portal::{DialState, ExternalReachability, PortalClient, PortalError, PortalStatus},
};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
struct ProbeTarget {
    identity: NetworkIdentity,
    connection: NetworkConnection,
    generation: u64,
    revision: u64,
}

#[derive(Default)]
struct State {
    report: Diagnostics,
    targets: [Option<ProbeTarget>; 2],
    revision: u64,
    generation: u64,
}

impl State {
    fn invalidate_connections(&mut self) {
        self.targets = [None, None];
        for role in Role::ALL {
            self.report.update(role, Update::Link(Link::Unknown));
            self.clear_observations(role);
        }
    }

    fn clear_observations(&mut self, role: Role) {
        self.report.update(role, Update::Auth(Auth::Unknown));
        self.report.update(role, Update::Carrier(Carrier::Unknown));
        self.report
            .update(role, Update::External(External::NotChecked));
        self.report.set_next_check(role, None);
    }
}

#[derive(Clone)]
pub(crate) struct ConnectionDiagnostics {
    inner: Arc<Mutex<State>>,
    generation: Arc<AtomicU64>,
    probe_running: Arc<AtomicBool>,
}

struct ActiveProbe(Arc<AtomicBool>);

impl Drop for ActiveProbe {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl ConnectionDiagnostics {
    pub fn new(generation: Arc<AtomicU64>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State::default())),
            generation,
            probe_running: Arc::new(AtomicBool::new(false)),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let generation = self.generation.load(Ordering::SeqCst);
        if generation != state.generation {
            state.generation = generation;
            state.invalidate_connections();
        }
        state
    }

    pub fn configure(&self, roles: &[(Role, Kind)]) {
        let mut state = self.state();
        state.report.configure(roles);
        state.targets = [None, None];
    }

    pub fn invalidate(&self) {
        self.state().invalidate_connections();
    }

    pub fn observe_link(
        &self,
        role: Role,
        identity: &NetworkIdentity,
        connection: ConnectionState,
        generation: u64,
    ) {
        let mut state = self.state();
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let link = match connection {
            ConnectionState::Ready(_) => Link::Matched,
            ConnectionState::DifferentEthernetAdapter => Link::AdapterChanged,
            ConnectionState::Unavailable => Link::Unavailable,
        };
        let previous = &state.targets[role.index()];
        let same = match (previous, connection.connection()) {
            (None, None) => true,
            (Some(previous), Some(connection)) => {
                previous.connection == connection
                    && previous.generation == generation
                    && previous.identity == *identity
            }
            _ => false,
        };
        state.report.update(role, Update::Link(link));
        if !same {
            state.revision = state.revision.wrapping_add(1);
            state.targets[role.index()] = connection.connection().map(|connection| ProbeTarget {
                identity: identity.clone(),
                connection,
                generation,
                revision: state.revision,
            });
            state.clear_observations(role);
        }
    }

    pub fn observe_portal(&self, role: Role, status: Result<PortalStatus, PortalError>) {
        let mut state = self.state();
        let current_generation = self.generation.load(Ordering::SeqCst);
        if state.targets[role.index()]
            .as_ref()
            .is_none_or(|target| target.generation != current_generation)
        {
            return;
        }
        let (auth, carrier) = match status {
            Ok(PortalStatus::Offline) => (Auth::Offline, Carrier::Unknown),
            Ok(PortalStatus::Online(dial)) => (
                Auth::Online,
                match dial {
                    DialState::Disabled => Carrier::NotRequired,
                    DialState::Pending => Carrier::Dialing,
                    DialState::Connected => Carrier::Connected,
                    DialState::Failed => Carrier::Failed,
                },
            ),
            Err(PortalError::NetworkChanged) => {
                state.targets[role.index()] = None;
                state.report.update(role, Update::Link(Link::Unknown));
                state.clear_observations(role);
                return;
            }
            Err(_) => (Auth::Unreachable, Carrier::Unknown),
        };
        state.report.update(role, Update::Auth(auth));
        state.report.update(role, Update::Carrier(carrier));
    }

    pub fn record(&self, role: Role, action: Action) {
        self.state().report.record(role, action);
    }

    pub fn next_check(&self, role: Role, duration: Option<Duration>) {
        let mut state = self.state();
        let duration = duration.filter(|_| state.targets[role.index()].is_some());
        state.report.set_next_check(role, duration);
    }

    pub fn summary(&self) -> String {
        self.state().report.summary()
    }

    pub fn report(&self) -> String {
        self.state().report.report()
    }

    pub fn check_external(&self) {
        if self.probe_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let ticket = ActiveProbe(Arc::clone(&self.probe_running));
        let generation = self.generation.load(Ordering::SeqCst);
        let targets = {
            let mut state = self.state();
            let targets = state
                .targets
                .clone()
                .map(|target| target.filter(|target| target.generation == generation));
            for (role, target) in Role::ALL.into_iter().zip(&targets) {
                state.report.update(
                    role,
                    Update::External(if target.is_some() {
                        External::Checking
                    } else {
                        External::Inconclusive
                    }),
                );
            }
            targets
        };
        if targets.iter().all(Option::is_none) {
            return;
        }
        let work = self.clone();
        let fallback = targets.clone();
        if thread::Builder::new()
            .name("campus-external-probe".into())
            .spawn(move || {
                let _ticket = ticket;
                for (role, target) in Role::ALL.into_iter().zip(targets) {
                    if let Some(target) = target {
                        let result = work.probe(role, &target);
                        work.finish_probe(role, &target, result);
                    }
                }
            })
            .is_err()
        {
            for (role, target) in Role::ALL.into_iter().zip(fallback) {
                if let Some(target) = target {
                    self.finish_probe(role, &target, ExternalReachability::Inconclusive);
                }
            }
        }
    }

    fn target_is_current(&self, role: Role, target: &ProbeTarget) -> bool {
        self.generation.load(Ordering::SeqCst) == target.generation
            && self.state().targets[role.index()].as_ref() == Some(target)
    }

    fn probe(&self, role: Role, target: &ProbeTarget) -> ExternalReachability {
        let ProbeTarget {
            identity,
            connection,
            ..
        } = target.clone();
        let view = self.clone();
        let target = target.clone();
        PortalClient::new(
            connection.ipv4,
            connection.interface_index,
            move || {
                view.target_is_current(role, &target)
                    && network::matched_connection(&identity).ok().flatten() == Some(connection)
            },
            Arc::new(Mutex::new(())),
        )
        .check_external()
    }

    fn finish_probe(&self, role: Role, target: &ProbeTarget, result: ExternalReachability) {
        let mut state = self.state();
        if state.targets[role.index()].as_ref() != Some(target) {
            return;
        }
        let state_changed = self.generation.load(Ordering::SeqCst) != target.generation;
        let external = match (state_changed, result) {
            (true, _) | (false, ExternalReachability::NetworkChanged) => External::NotChecked,
            (false, ExternalReachability::Reachable) => External::Reachable,
            (false, ExternalReachability::PortalRedirect) => External::PortalRedirect,
            (false, ExternalReachability::Inconclusive) => External::Inconclusive,
        };
        state.report.update(role, Update::External(external));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn identity() -> NetworkIdentity {
        NetworkIdentity {
            adapter_id: "private-adapter-id".into(),
            gateway: Ipv4Addr::new(192, 0, 2, 1),
            ssid: Some(b"private-ssid".to_vec()),
        }
    }

    fn connection(ip: u8) -> NetworkConnection {
        NetworkConnection {
            ipv4: Ipv4Addr::new(192, 0, 2, ip),
            interface_index: 1,
        }
    }

    fn new() -> ConnectionDiagnostics {
        let view = ConnectionDiagnostics::new(Arc::new(AtomicU64::new(10)));
        view.configure(&[
            (Role::Primary, Kind::Wifi),
            (Role::Secondary, Kind::Ethernet),
        ]);
        view
    }

    #[test]
    fn a_portal_and_carrier_success_does_not_assert_external_reachability() {
        let view = new();
        view.observe_link(
            Role::Primary,
            &identity(),
            ConnectionState::Ready(connection(10)),
            10,
        );
        view.observe_portal(
            Role::Primary,
            Ok(PortalStatus::Online(DialState::Connected)),
        );
        let summary = view.summary();
        assert!(summary.contains("已认证"), "{summary}");
        assert!(summary.contains("已拨号"), "{summary}");
        assert!(summary.contains("未检测"), "{summary}");
        assert!(!view.report().contains("private"));
        assert!(!view.report().contains("192.0.2"));
    }

    #[test]
    fn a_probe_for_an_older_interface_address_does_not_replace_current_status() {
        let view = new();
        let identity = identity();
        view.observe_link(
            Role::Primary,
            &identity,
            ConnectionState::Ready(connection(10)),
            10,
        );
        let old = view.inner.lock().unwrap().targets[0].clone().unwrap();
        view.observe_link(
            Role::Primary,
            &identity,
            ConnectionState::Ready(connection(20)),
            10,
        );
        view.finish_probe(Role::Primary, &old, ExternalReachability::Reachable);
        assert!(!view.summary().contains("可达"));
        let current = view.inner.lock().unwrap().targets[0].clone().unwrap();
        view.finish_probe(Role::Primary, &current, ExternalReachability::Reachable);
        assert!(view.summary().contains("可达"));
    }

    #[test]
    fn power_or_configuration_generation_rejects_results_even_before_worker_clears_old_targets() {
        let view = new();
        view.observe_link(
            Role::Primary,
            &identity(),
            ConnectionState::Ready(connection(10)),
            10,
        );
        view.observe_portal(
            Role::Primary,
            Ok(PortalStatus::Online(DialState::Connected)),
        );
        let old = view.inner.lock().unwrap().targets[0].clone().unwrap();
        view.generation.store(11, Ordering::SeqCst);
        assert!(!view.summary().contains("已认证"));
        view.finish_probe(Role::Primary, &old, ExternalReachability::Reachable);
        assert!(!view.summary().contains("可达"));
    }

    #[test]
    fn a_disconnect_and_reconnect_with_the_same_address_invalidates_the_old_probe() {
        let view = new();
        let identity = identity();
        let ready = ConnectionState::Ready(connection(10));
        view.observe_link(Role::Primary, &identity, ready, 10);
        let old = view.inner.lock().unwrap().targets[0].clone().unwrap();
        view.observe_link(Role::Primary, &identity, ConnectionState::Unavailable, 10);
        view.observe_link(Role::Primary, &identity, ready, 10);
        view.finish_probe(Role::Primary, &old, ExternalReachability::Reachable);
        assert!(!view.summary().contains("可达"));
    }
}
