#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analytics;
mod update;
mod windows_ui;
mod winhttp;

use campus_network_guardian::{
    controller::{Action, Controller},
    network,
    portal::{DialState, PortalClient, PortalError, PortalStatus},
    routing::{RoutePreference, RouteTarget},
    wifi_mac,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows_ui::{Event, Settings};

#[derive(Clone, Serialize, Deserialize)]
struct SavedConfig {
    identity: network::NetworkIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secondary_identity: Option<network::NetworkIdentity>,
    username: String,
    http_acknowledged: bool,
    autostart: bool,
}

impl SavedConfig {
    fn identities(&self) -> impl Iterator<Item = &network::NetworkIdentity> {
        std::iter::once(&self.identity).chain(self.secondary_identity.as_ref())
    }

    fn distinct_adapters(&self) -> bool {
        self.secondary_identity
            .as_ref()
            .is_none_or(|secondary| self.identity.adapter_id != secondary.adapter_id)
    }

    fn from_settings(
        settings: Settings,
        previous: Option<&Self>,
        is_connected: impl Fn(&network::NetworkIdentity) -> bool,
    ) -> Result<Self, &'static str> {
        const CHANGED: &str = "网卡或网络身份已变化，请重新选择并确认";
        let candidate = Self {
            identity: serde_json::from_str(&settings.adapter_id).map_err(|_| CHANGED)?,
            secondary_identity: if settings.secondary_adapter_id.is_empty() {
                None
            } else {
                Some(serde_json::from_str(&settings.secondary_adapter_id).map_err(|_| CHANGED)?)
            },
            username: settings.username,
            http_acknowledged: settings.http_acknowledged,
            autostart: settings.autostart,
        };
        if !candidate.distinct_adapters() {
            return Err("请选择两张不同的网卡");
        }
        if !candidate.identities().all(|identity| {
            previous.is_some_and(|saved| saved.identities().any(|known| known == identity))
                || is_connected(identity)
        }) {
            return Err(CHANGED);
        }
        Ok(candidate)
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| {
        PathBuf::from(p)
            .join("CampusNetworkGuardian")
            .join("config.json")
    })
}
fn load_config() -> Option<SavedConfig> {
    let bytes = fs::read(config_path()?).ok()?;
    let config: SavedConfig = serde_json::from_slice(&bytes).ok()?;
    (config.http_acknowledged && !config.username.trim().is_empty() && config.distinct_adapters())
        .then_some(config)
}
fn save_config(config: &SavedConfig) -> Result<(), String> {
    let path = config_path().ok_or("无法确定配置目录")?;
    fs::create_dir_all(path.parent().ok_or("配置目录无效")?).map_err(|_| "无法创建配置目录")?;
    let data = serde_json::to_vec(config).map_err(|_| "无法生成配置")?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, data).map_err(|_| "无法保存配置".to_string())?;
    let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let _ = fs::remove_file(temp);
        return Err("无法替换配置文件".to_string());
    }
    Ok(())
}
fn display(status: &Arc<Mutex<String>>, message: &str) {
    if let Ok(mut value) = status.lock() {
        *value = message.to_string();
    }
}
fn ui_settings(config: &Option<SavedConfig>) -> Settings {
    config
        .as_ref()
        .map(|c| Settings {
            adapter_id: serde_json::to_string(&c.identity).unwrap_or_default(),
            secondary_adapter_id: c
                .secondary_identity
                .as_ref()
                .and_then(|identity| serde_json::to_string(identity).ok())
                .unwrap_or_default(),
            username: c.username.clone(),
            http_acknowledged: c.http_acknowledged,
            autostart: c.autostart,
        })
        .unwrap_or_default()
}

fn erase_password(password: &mut String) {
    unsafe {
        for byte in password.as_bytes_mut() {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
}

fn configured_networks(
    config: &SavedConfig,
    now: Instant,
    credentials_available: bool,
) -> Vec<GuardedNetwork> {
    config
        .identities()
        .map(|identity| {
            let mut network = GuardedNetwork::new(identity.clone(), now);
            if !credentials_available {
                network.controller.credentials_deleted();
                network.message = network.controller.message();
            }
            network
        })
        .collect()
}

fn startup_networks(
    now: Instant,
    config: Option<&SavedConfig>,
    read_password: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Vec<GuardedNetwork> {
    let Some(saved) = config else {
        return Vec::new();
    };
    let usable = if saved.http_acknowledged && !saved.username.trim().is_empty() {
        match read_password(&saved.username) {
            Ok(Some(mut password)) => {
                let present = !password.is_empty();
                erase_password(&mut password);
                present
            }
            _ => false,
        }
    } else {
        false
    };
    configured_networks(saved, now, usable)
}

fn worker_wait(
    now: Instant,
    next: Instant,
    connected: bool,
    mac_retry_at: Option<Instant>,
) -> Duration {
    if connected {
        next.max(mac_retry_at.unwrap_or(next))
            .saturating_duration_since(now)
            .min(Duration::from_secs(5))
    } else {
        Duration::from_secs(5)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteRole {
    Isp,
    Ethernet,
}

fn preferred_campus_route(
    isp: Option<PortalStatus>,
    ethernet: Option<PortalStatus>,
) -> Option<RouteRole> {
    let usable = |status| {
        matches!(
            status,
            Some(PortalStatus::Online(
                DialState::Connected | DialState::Disabled
            ))
        )
    };
    if usable(isp) {
        Some(RouteRole::Isp)
    } else if usable(ethernet) {
        Some(RouteRole::Ethernet)
    } else {
        None
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn fake_config() -> SavedConfig {
        SavedConfig {
            identity: network::NetworkIdentity {
                adapter_id: "fake-adapter".into(),
                gateway: Ipv4Addr::new(10, 0, 0, 1),
                ssid: None,
            },
            secondary_identity: None,
            username: "fake-user".into(),
            http_acknowledged: true,
            autostart: false,
        }
    }

    #[test]
    fn explicitly_saved_ethernet_identity_survives_loading_with_wifi() {
        let ethernet = fake_config().identity;
        let wifi = network::NetworkIdentity {
            adapter_id: "fake-wireless-adapter".into(),
            ssid: Some(b"WHUT-ISP".to_vec()),
            ..ethernet.clone()
        };
        let saved = serde_json::json!({
            "identity": wifi,
            "secondary_identity": ethernet,
            "username": "fake-user",
            "http_acknowledged": true,
            "autostart": false
        });
        let config: SavedConfig = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(serde_json::to_value(&config).unwrap(), saved);
    }

    #[test]
    fn legacy_config_continues_to_guard_only_its_saved_network() {
        let old = serde_json::json!({
            "identity": fake_config().identity,
            "username": "fake-user",
            "http_acknowledged": true,
            "autostart": false
        });
        let loaded: SavedConfig = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), old);
        assert_eq!(
            startup_networks(Instant::now(), Some(&loaded), |_| Ok(Some("test".into()))).len(),
            1
        );
    }

    #[test]
    fn online_wifi_does_not_block_login_on_saved_ethernet() {
        let now = Instant::now();
        let mut config = fake_config();
        config.secondary_identity = Some(config.identity.clone());
        config.identity.adapter_id = "wifi-adapter".into();
        config.identity.ssid = Some(b"WHUT-ISP".to_vec());
        let mut networks = startup_networks(now, Some(&config), |_| Ok(Some("test".into())));
        let [wifi, ethernet] = networks.as_mut_slice() else {
            panic!("both saved networks must have a controller");
        };
        assert!(wifi.identity.ssid.is_some());
        assert!(ethernet.identity.ssid.is_none());
        assert_eq!(
            wifi.observe_status(now, Ok(PortalStatus::Online(DialState::Connected))),
            Action::None
        );
        assert_eq!(
            ethernet.observe_status(now, Ok(PortalStatus::Offline)),
            Action::Login
        );

        let status = Arc::new(Mutex::new(String::new()));
        display_networks(&status, &networks, None);
        let status = status.lock().unwrap();
        assert!(status.starts_with("Wi-Fi：校园网在线\r\n以太网："));
    }

    #[test]
    fn routing_prefers_isp_and_falls_back_to_online_ethernet() {
        let online = Some(PortalStatus::Online(DialState::Connected));
        let wired = Some(PortalStatus::Online(DialState::Disabled));
        assert_eq!(preferred_campus_route(online, wired), Some(RouteRole::Isp));
        for unavailable in [
            None,
            Some(PortalStatus::Offline),
            Some(PortalStatus::Online(DialState::Failed)),
            Some(PortalStatus::Online(DialState::Pending)),
        ] {
            assert_eq!(
                preferred_campus_route(unavailable, wired),
                Some(RouteRole::Ethernet)
            );
            assert_eq!(preferred_campus_route(unavailable, None), None);
        }
        assert_eq!(preferred_campus_route(online, None), Some(RouteRole::Isp));
    }

    #[test]
    fn routing_uses_recent_portal_state_through_a_transient_error() {
        let now = Instant::now();
        let online = PortalStatus::Online(DialState::Connected);
        let mut link = GuardedNetwork::new(fake_config().identity, now);
        link.observe_status(now, Ok(online));
        let retry = now + Duration::from_secs(60);
        link.observe_status(retry, Err(PortalError::Network));
        assert_eq!(link.current_route_status(retry), Some(online));
        assert_eq!(
            link.current_route_status(now + Duration::from_secs(90)),
            None
        );
        link.network_changed(retry);
        assert_eq!(link.current_route_status(retry), None);
    }

    #[test]
    fn second_adapter_requires_connection_to_enroll_but_can_be_retained_offline() {
        let primary = fake_config();
        let mut settings = ui_settings(&Some(primary.clone()));
        let mut secondary = primary.identity.clone();
        secondary.adapter_id = "other-adapter".into();
        settings.secondary_adapter_id = serde_json::to_string(&secondary).unwrap();
        assert!(SavedConfig::from_settings(settings.clone(), Some(&primary), |_| false).is_err());
        let enrolled = SavedConfig::from_settings(settings.clone(), Some(&primary), |identity| {
            *identity == secondary
        })
        .unwrap();
        assert_eq!(enrolled.secondary_identity, Some(secondary.clone()));
        assert!(SavedConfig::from_settings(settings.clone(), Some(&enrolled), |_| false).is_ok());
        settings.secondary_adapter_id.clear();
        let removed = SavedConfig::from_settings(settings, Some(&enrolled), |_| false).unwrap();
        assert!(removed.secondary_identity.is_none());
    }

    #[test]
    fn two_fingerprints_of_the_same_adapter_cannot_be_saved_together() {
        let primary = fake_config();
        let mut settings = ui_settings(&Some(primary.clone()));
        let mut secondary = primary.identity.clone();
        secondary.gateway = Ipv4Addr::new(10, 1, 0, 1);
        settings.secondary_adapter_id = serde_json::to_string(&secondary).unwrap();
        assert!(SavedConfig::from_settings(settings, Some(&primary), |_| true).is_err());
    }

    #[test]
    fn missing_startup_credentials_must_block_redial() {
        let now = Instant::now();
        let config = fake_config();
        let mut networks = startup_networks(now, Some(&config), |_| Ok(None));
        let controller = &mut networks[0].controller;
        assert!(
            !controller.configuration_available(),
            "missing credentials must also block Wi-Fi profile writes"
        );
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Offline)),
            Action::None
        );
        controller.configured(now);
        assert!(controller.configuration_available());
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
    }

    #[test]
    fn startup_handles_unreadable_or_empty_credentials_without_retrying() {
        let now = Instant::now();
        let config = fake_config();
        for result in [Err("not accessible".to_owned()), Ok(Some(String::new()))] {
            let mut networks = startup_networks(now, Some(&config), |_| result);
            let controller = &mut networks[0].controller;
            assert_eq!(
                controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
                Action::None
            );
            assert_eq!(
                controller.on_status(now, Ok(PortalStatus::Offline)),
                Action::None
            );
        }
    }

    #[test]
    fn valid_startup_credentials_preserve_automatic_actions() {
        let now = Instant::now();
        let config = fake_config();
        let mut networks = startup_networks(now, Some(&config), |username| {
            assert_eq!(username, "fake-user");
            Ok(Some("fake-password".into()))
        });
        let controller = &mut networks[0].controller;
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Offline)),
            Action::Login
        );
    }

    #[test]
    fn startup_does_not_read_credentials_without_accepted_configuration() {
        let now = Instant::now();
        let mut config = fake_config();
        config.http_acknowledged = false;
        let mut networks =
            startup_networks(now, Some(&config), |_| panic!("must not read credentials"));
        let controller = &mut networks[0].controller;
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
        assert!(startup_networks(now, None, |_| panic!("must not read credentials")).is_empty());
    }

    #[test]
    fn mac_wait_and_network_settling_never_turn_into_a_busy_poll_loop() {
        let now = Instant::now();
        let expired = now - Duration::from_secs(1);
        let settling = now + Duration::from_secs(5);
        assert_eq!(
            worker_wait(now, expired, true, Some(settling)),
            Duration::from_secs(5)
        );
        assert_eq!(
            worker_wait(now, settling, true, Some(expired)),
            Duration::from_secs(5)
        );
        assert_eq!(
            worker_wait(now, expired, false, Some(expired)),
            Duration::from_secs(5)
        );
    }
}

struct CredentialsUnavailable;

struct GuardedNetwork {
    identity: network::NetworkIdentity,
    controller: Controller,
    last_connection: Option<network::NetworkConnection>,
    mac_guard: wifi_mac::Guard,
    mac_retry_at: Option<Instant>,
    route_status: Option<(Instant, PortalStatus)>,
    message: &'static str,
}

impl GuardedNetwork {
    fn new(identity: network::NetworkIdentity, now: Instant) -> Self {
        let controller = Controller::new(now);
        Self {
            identity,
            message: controller.message(),
            controller,
            last_connection: None,
            mac_guard: wifi_mac::Guard::default(),
            mac_retry_at: None,
            route_status: None,
        }
    }

    fn wait(&self, now: Instant) -> Duration {
        worker_wait(
            now,
            self.controller.next(),
            self.last_connection.is_some(),
            self.mac_retry_at,
        )
    }

    fn network_changed(&mut self, now: Instant) {
        self.last_connection = None;
        self.mac_retry_at = None;
        self.route_status = None;
        self.controller.network_changed(now);
        self.message = self.controller.message();
    }

    fn pause(&mut self, message: &'static str) {
        self.controller.credentials_deleted();
        self.route_status = None;
        self.message = message;
    }

    fn observe_status(
        &mut self,
        now: Instant,
        status: Result<PortalStatus, PortalError>,
    ) -> Action {
        if let Ok(status) = status {
            self.route_status = Some((now, status));
        } else if status == Err(PortalError::NetworkChanged) {
            self.route_status = None;
        }
        let action = self.controller.on_status(now, status);
        self.message = self.controller.message();
        action
    }

    fn current_route_status(&self, now: Instant) -> Option<PortalStatus> {
        const STATUS_LIFETIME: Duration = Duration::from_secs(90);
        self.route_status
            .filter(|(observed, _)| now.saturating_duration_since(*observed) < STATUS_LIFETIME)
            .map(|(_, status)| status)
    }

    fn route_target(&self) -> Option<RouteTarget> {
        let connection = self.last_connection?;
        (network::matched_connection(&self.identity).ok().flatten() == Some(connection)).then_some(
            RouteTarget {
                connection,
                gateway: self.identity.gateway,
            },
        )
    }

    fn check(
        &mut self,
        username: &str,
        applied_generation: u64,
        generation: &Arc<AtomicU64>,
        gate: &Arc<Mutex<()>>,
    ) -> Result<(), CredentialsUnavailable> {
        let state = network::inspect_connection(&self.identity)
            .unwrap_or(network::ConnectionState::Unavailable);
        let connection = state.connection();
        if connection != self.last_connection {
            self.network_changed(Instant::now());
            self.last_connection = connection;
        }
        let Some(connection) = connection else {
            self.message = match state {
                network::ConnectionState::DifferentEthernetAdapter => {
                    "以太网网卡已变化，自动操作已暂停。请在设置中重新选择校园网网卡"
                }
                _ => "校园网络不匹配或不可用，已暂停",
            };
            return Ok(());
        };
        let now = Instant::now();
        if !self.controller.due(now) || self.mac_retry_at.is_some_and(|next| now < next) {
            return Ok(());
        }
        if !self.controller.configuration_available() {
            self.message = self.controller.message();
            self.mac_retry_at = Some(now + Duration::from_secs(30));
            return Ok(());
        }
        // Profile changes are serialized with UI configuration invalidation. The
        // WLAN backend also rechecks the selected identity immediately before writing.
        let readiness = match gate.lock() {
            Ok(_lock) if generation.load(Ordering::SeqCst) == applied_generation => {
                self.mac_guard.prepare(&self.identity, connection)
            }
            _ => wifi_mac::Readiness::Manual(wifi_mac::Error::Changed),
        };
        if readiness != wifi_mac::Readiness::Ready {
            self.route_status = None;
            self.message = readiness.message();
            self.mac_retry_at = Some(
                Instant::now()
                    + Duration::from_secs(
                        if matches!(readiness, wifi_mac::Readiness::WaitingForReconnect) {
                            5
                        } else {
                            30
                        },
                    ),
            );
            return Ok(());
        }
        self.mac_retry_at = None;
        let mac_proof = self.mac_guard.proof();
        let post_mac_proof = mac_proof.clone();
        let identity = self.identity.clone();
        let current_generation = Arc::clone(generation);
        let portal = PortalClient::new(
            connection.ipv4,
            connection.interface_index,
            move || {
                current_generation.load(Ordering::SeqCst) == applied_generation
                    && network::matched_connection(&identity).ok().flatten() == Some(connection)
                    && wifi_mac::authentication_ready(&identity, connection, &post_mac_proof)
            },
            Arc::clone(gate),
        );
        let portal_status = portal.status();
        let action = self.observe_status(Instant::now(), portal_status);
        if action == Action::NetworkChanged {
            self.last_connection = None;
            return Ok(());
        }
        if !matches!(action, Action::Login | Action::Redial) {
            return Ok(());
        }
        if generation.load(Ordering::SeqCst) != applied_generation
            || network::matched_connection(&self.identity).ok().flatten() != Some(connection)
            || !wifi_mac::authentication_ready(&self.identity, connection, &mac_proof)
        {
            self.network_changed(Instant::now());
            return Ok(());
        }
        let outcome = match action {
            Action::Login => match windows_ui::read_password(username) {
                Ok(Some(mut password)) if !password.is_empty() => {
                    let result = portal.login(username, &password).map(Some);
                    erase_password(&mut password);
                    result
                }
                _ => return Err(CredentialsUnavailable),
            },
            Action::Redial => portal.redial().map(|()| None),
            _ => unreachable!(),
        };
        let now = Instant::now();
        match outcome {
            Err(PortalError::NetworkChanged) => {
                self.last_connection = None;
                self.route_status = None;
            }
            Ok(Some(status)) => self.route_status = Some((now, status)),
            _ => {}
        }
        self.controller.on_action_result(now, action, outcome);
        self.message = self.controller.message();
        Ok(())
    }
}

fn apply_campus_route_preference(
    routing: &mut RoutePreference,
    networks: &[GuardedNetwork],
    now: Instant,
    can_adjust_routes: bool,
) -> Option<&'static str> {
    let isp = networks
        .iter()
        .find(|n| n.identity.ssid.as_deref() == Some(b"WHUT-ISP"));
    let ethernet = networks.iter().find(|n| n.identity.ssid.is_none());
    let targets = match (isp, ethernet) {
        (Some(isp), Some(ethernet)) => Some((isp, ethernet)),
        _ => None,
    };
    if targets.is_some() && !can_adjust_routes {
        return Some("自动切换需以管理员身份运行");
    }
    let (selection, preferred, lone_connection) = if let Some((isp, ethernet)) = targets {
        let wifi_target = isp.route_target();
        let ethernet_target = ethernet.route_target();
        let role = preferred_campus_route(
            wifi_target.and_then(|_| isp.current_route_status(now)),
            ethernet_target.and_then(|_| ethernet.current_route_status(now)),
        );
        let (selection, preferred) = match role {
            Some(RouteRole::Isp) => (
                wifi_target.zip(ethernet_target),
                Some("校园网 IPv4 首选 WHUT-ISP"),
            ),
            Some(RouteRole::Ethernet) => (
                ethernet_target.zip(wifi_target),
                Some("校园网 IPv4 由以太网兜底"),
            ),
            None => (None, None),
        };
        let lone_connection = match (role, wifi_target, ethernet_target) {
            (Some(RouteRole::Isp), Some(_), None) => Some("WHUT-ISP 可用，等待以太网"),
            (Some(RouteRole::Ethernet), None, Some(_)) => Some("以太网可用，等待 WHUT-ISP"),
            _ => None,
        };
        (selection, preferred, lone_connection)
    } else {
        (None, None, None)
    };
    match routing.update(selection) {
        Ok(true) => preferred,
        Ok(false) if targets.is_none() => None,
        Ok(false) if lone_connection.is_some() => lone_connection,
        Ok(false) if preferred.is_some() => Some("沿用 Windows 当前路由"),
        Ok(false) => Some("等待校园网认证状态"),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            Some("需要以管理员身份运行才能自动切换")
        }
        Err(_) => Some("暂时无法调整路由"),
    }
}

fn display_networks(
    status: &Arc<Mutex<String>>,
    networks: &[GuardedNetwork],
    routing: Option<&str>,
) {
    match networks {
        [] => display(status, "请先完成设置"),
        [network] => display(status, network.message),
        [first, second, ..] => {
            let labels = match (
                first.identity.ssid.is_some(),
                second.identity.ssid.is_some(),
            ) {
                (true, false) => ("Wi-Fi", "以太网"),
                (false, true) => ("以太网", "Wi-Fi"),
                _ => ("主网络", "第二个网络"),
            };
            let mut message = format!(
                "{}：{}\r\n{}：{}",
                labels.0, first.message, labels.1, second.message
            );
            if let Some(routing) = routing {
                message.push_str("\r\n系统选路：");
                message.push_str(routing);
            }
            display(status, &message);
        }
    }
}

fn worker(
    rx: mpsc::Receiver<Event>,
    status: Arc<Mutex<String>>,
    mut config: Option<SavedConfig>,
    generation: Arc<AtomicU64>,
    gate: Arc<Mutex<()>>,
    routing: Arc<Mutex<Option<RoutePreference>>>,
) {
    let mut networks = startup_networks(Instant::now(), config.as_ref(), windows_ui::read_password);
    let can_adjust_routes = unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } != 0;
    let mut route_message = None;
    display_networks(&status, &networks, route_message);
    let mut applied_generation = generation.load(Ordering::SeqCst);
    loop {
        let now = Instant::now();
        let timeout = if generation.load(Ordering::SeqCst) == applied_generation {
            networks
                .iter()
                .map(|network| network.wait(now))
                .min()
                .unwrap_or(Duration::from_secs(5))
        } else {
            Duration::from_secs(5)
        };
        match rx.recv_timeout(timeout) {
            Ok(Event::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Event::SettingsSaved(settings, reply)) => {
                let candidate = SavedConfig::from_settings(settings, config.as_ref(), |identity| {
                    network::matched_connection(identity)
                        .ok()
                        .flatten()
                        .is_some()
                });
                let result = candidate.and_then(|candidate| {
                    save_config(&candidate)
                        .map_err(|_| "无法保存配置，已尝试恢复原凭据和自启动设置")?;
                    let now = Instant::now();
                    let mut previous = std::mem::take(&mut networks);
                    networks = candidate
                        .identities()
                        .map(|identity| {
                            // Keep an uncertain POST's observation window for a retained network.
                            let mut network = previous
                                .iter()
                                .position(|network| network.identity == *identity)
                                .map(|index| previous.swap_remove(index))
                                .unwrap_or_else(|| GuardedNetwork::new(identity.clone(), now));
                            network.controller.configured(now);
                            network.network_changed(now);
                            network.mac_guard = wifi_mac::Guard::default();
                            network
                        })
                        .collect();
                    config = Some(candidate);
                    applied_generation = generation.load(Ordering::SeqCst);
                    display_networks(&status, &networks, None);
                    Ok(())
                });
                if let Err(message) = result {
                    for network in &mut networks {
                        network.pause(message);
                    }
                    display(&status, message);
                }
                let _ = reply.send(result.map_err(str::to_owned));
            }
            Ok(Event::CredentialsDeleted) => {
                applied_generation = generation.load(Ordering::SeqCst);
                for network in &mut networks {
                    network.pause("密码已删除，自动操作已暂停");
                }
            }
            Ok(Event::SettingsFailed) => {
                applied_generation = generation.load(Ordering::SeqCst);
                for network in &mut networks {
                    network.pause("设置失败，自动操作已暂停，请重新保存设置");
                }
            }
            Ok(Event::NetworkChanged(event_generation)) => {
                applied_generation = event_generation;
                for network in &mut networks {
                    network.network_changed(Instant::now());
                }
            }
            Ok(Event::CheckNow) => {
                for network in &mut networks {
                    network.mac_retry_at = None;
                    network.controller.check_now(Instant::now());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(saved) = config.as_ref() {
            let mut credentials_unavailable = false;
            for network in &mut networks {
                if generation.load(Ordering::SeqCst) != applied_generation {
                    break;
                }
                if network
                    .check(&saved.username, applied_generation, &generation, &gate)
                    .is_err()
                {
                    credentials_unavailable = true;
                    break;
                }
            }
            if credentials_unavailable {
                for network in &mut networks {
                    network.pause("密码不可用，自动操作已暂停，请打开设置");
                }
            }
        }
        if let Ok(_lock) = gate.lock() {
            if generation.load(Ordering::SeqCst) == applied_generation {
                if let Ok(mut preference) = routing.lock() {
                    if let Some(preference) = preference.as_mut() {
                        route_message = apply_campus_route_preference(
                            preference,
                            &networks,
                            Instant::now(),
                            can_adjust_routes,
                        );
                    }
                }
            }
        }
        display_networks(&status, &networks, route_message);
    }
}

fn main() {
    if !matches!(windows_ui::single_instance(), Ok(true)) {
        if std::env::args().any(|arg| arg == "--show") {
            windows_ui::show_existing_panel();
        }
        return;
    }
    let config = load_config();
    let mut initial = ui_settings(&config);
    // The installer can set Run before the first configuration exists. Reflect
    // the actual Windows setting instead of overwriting it with a stale flag.
    initial.autostart = windows_ui::autostart_enabled().unwrap_or(initial.autostart);
    let status = Arc::new(Mutex::new("正在启动".to_string()));
    let generation = Arc::new(AtomicU64::new(0));
    let gate = Arc::new(Mutex::new(()));
    let (tx, rx) = mpsc::channel();
    let worker_status = Arc::clone(&status);
    let worker_generation = Arc::clone(&generation);
    let worker_gate = Arc::clone(&gate);
    let routing = Arc::new(Mutex::new(Some(RoutePreference::default())));
    let worker_routing = Arc::clone(&routing);
    thread::spawn(move || {
        worker(
            rx,
            worker_status,
            config,
            worker_generation,
            worker_gate,
            worker_routing,
        )
    });
    let adapters = Box::new(|| {
        network::list_adapters()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| {
                let id = serde_json::to_string(&a.identity).ok()?;
                let network_name = a
                    .identity
                    .ssid
                    .as_ref()
                    .map(|s| format!(" / Wi-Fi: {}", String::from_utf8_lossy(s)))
                    .unwrap_or_default();
                Some(windows_ui::Adapter {
                    id,
                    name: format!("{}{} / 网关 {}", a.name, network_name, a.identity.gateway),
                })
            })
            .collect()
    });
    let updates = update::start();
    let update_status = updates.clone();
    let update_info = updates.clone();
    let analytics = analytics::start();
    let analytics_status = analytics.clone();
    let _ = windows_ui::run(
        initial,
        adapters,
        tx,
        status,
        generation,
        gate,
        windows_ui::UiUpdates {
            request: Box::new(move || {
                updates.request_check();
            }),
            status: Box::new(move || match update_status.status() {
                update::UpdateStatus::Idle => windows_ui::UpdateState::Idle,
                update::UpdateStatus::Checking => windows_ui::UpdateState::Checking,
                update::UpdateStatus::Current => windows_ui::UpdateState::Current,
                update::UpdateStatus::Installing => windows_ui::UpdateState::Installing,
                update::UpdateStatus::Failed => windows_ui::UpdateState::Failed,
            }),
            info: Box::new(move || {
                let info = update_info.info();
                windows_ui::UpdateInfo {
                    version: info.version,
                    source: info.source,
                    changelog: info.changelog,
                    progress: info.progress,
                }
            }),
        },
        windows_ui::UiAnalytics {
            enabled: Box::new(move || analytics_status.enabled()),
            set_enabled: Box::new(move |enabled| analytics.set_enabled(enabled)),
        },
    );
    // Restore temporary routing as the UI exits, even when a portal GET is
    // still waiting on the worker. Taking the state prevents any later writes.
    let preference = routing.lock().unwrap_or_else(|err| err.into_inner()).take();
    if let Some(mut preference) = preference {
        let _ = preference.update(None);
    }
}
