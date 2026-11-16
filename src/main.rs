#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod windows_ui;

use campus_network_guardian::{
    controller::{Action, Controller},
    network,
    portal::{PortalClient, PortalError},
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
    username: String,
    http_acknowledged: bool,
    autostart: bool,
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
    (config.http_acknowledged && !config.username.trim().is_empty()).then_some(config)
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

fn startup_controller(
    now: Instant,
    config: Option<&SavedConfig>,
    read_password: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Controller {
    let mut controller = Controller::new(now);
    if let Some(saved) = config {
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
        if !usable {
            controller.credentials_deleted();
        }
    }
    controller
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    use campus_network_guardian::portal::{DialState, PortalStatus};
    use std::net::Ipv4Addr;

    fn fake_config() -> SavedConfig {
        SavedConfig {
            identity: network::NetworkIdentity {
                adapter_id: "fake-adapter".into(),
                gateway: Ipv4Addr::new(10, 0, 0, 1),
                ssid: None,
            },
            username: "fake-user".into(),
            http_acknowledged: true,
            autostart: false,
        }
    }

    #[test]
    fn missing_startup_credentials_must_block_redial() {
        let now = Instant::now();
        let config = fake_config();
        let mut controller = startup_controller(now, Some(&config), |_| Ok(None));
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Offline)),
            Action::None
        );
        controller.configured(now);
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
            let mut controller = startup_controller(now, Some(&config), |_| result);
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
        let mut controller = startup_controller(now, Some(&config), |username| {
            assert_eq!(username, "fake-user");
            Ok(Some("fake-password".into()))
        });
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
        let mut controller =
            startup_controller(now, Some(&config), |_| panic!("must not read credentials"));
        assert_eq!(
            controller.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
        let _ = startup_controller(now, None, |_| panic!("must not read credentials"));
    }
}

fn worker(
    rx: mpsc::Receiver<Event>,
    status: Arc<Mutex<String>>,
    mut config: Option<SavedConfig>,
    generation: Arc<AtomicU64>,
    gate: Arc<Mutex<()>>,
) {
    let mut controller =
        startup_controller(Instant::now(), config.as_ref(), windows_ui::read_password);
    if config.is_some() {
        display(&status, controller.message());
    }
    let mut last_connection: Option<network::NetworkConnection> = None;
    let mut applied_generation = generation.load(Ordering::SeqCst);
    loop {
        let timeout = if last_connection.is_some() {
            controller
                .next()
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(5))
        } else {
            Duration::from_secs(5)
        };
        match rx.recv_timeout(timeout) {
            Ok(Event::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Event::SettingsSaved(settings, reply)) => {
                let identity: Option<network::NetworkIdentity> =
                    serde_json::from_str(&settings.adapter_id).ok();
                let valid = identity
                    .as_ref()
                    .is_some_and(|id| network::matched_connection(id).ok().flatten().is_some());
                if let Some(identity) = identity.filter(|_| valid) {
                    let candidate = SavedConfig {
                        identity,
                        username: settings.username,
                        http_acknowledged: settings.http_acknowledged,
                        autostart: settings.autostart,
                    };
                    if save_config(&candidate).is_ok() {
                        config = Some(candidate);
                        controller.configured(Instant::now());
                        applied_generation = generation.load(Ordering::SeqCst);
                        last_connection = None;
                        let _ = reply.send(Ok(()));
                    } else {
                        controller.credentials_deleted();
                        display(&status, "配置保存失败，请重新打开设置");
                        let _ =
                            reply.send(Err("无法保存配置，已尝试恢复原凭据和自启动设置".into()));
                    }
                } else {
                    controller.credentials_deleted();
                    display(&status, "网卡已变化，请重新打开设置");
                    let _ = reply.send(Err("网卡或网络身份已变化，请重新选择并确认".into()));
                }
            }
            Ok(Event::CredentialsDeleted) => {
                controller.credentials_deleted();
                display(&status, "密码已删除，自动操作已暂停");
            }
            Ok(Event::SettingsFailed) => {
                controller.credentials_deleted();
                display(&status, "设置失败，自动操作已暂停，请重新保存设置");
            }
            Ok(Event::NetworkChanged(event_generation)) => {
                applied_generation = event_generation;
                last_connection = None;
                controller.network_changed(Instant::now());
            }
            Ok(Event::CheckNow) => {
                controller.check_now(Instant::now());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let Some(ref saved) = config else {
            display(&status, "请先完成设置");
            continue;
        };
        let connection = network::matched_connection(&saved.identity).ok().flatten();
        if connection != last_connection {
            last_connection = connection;
            controller.network_changed(Instant::now());
        }
        let Some(connection) = connection else {
            display(&status, "校园网络不匹配或不可用，已暂停");
            continue;
        };
        if !controller.due(Instant::now()) {
            continue;
        }
        let identity = saved.identity.clone();
        let current_generation = Arc::clone(&generation);
        let post_gate = Arc::clone(&gate);
        let portal = PortalClient::new(
            connection.ipv4,
            connection.interface_index,
            move || {
                current_generation.load(Ordering::SeqCst) == applied_generation
                    && network::matched_connection(&identity).ok().flatten() == Some(connection)
            },
            post_gate,
        );
        let portal_status = portal.status();
        let action = controller.on_status(Instant::now(), portal_status);
        display(&status, controller.message());
        if action == Action::NetworkChanged {
            last_connection = None;
            continue;
        }
        if !matches!(action, Action::Login | Action::Redial) {
            continue;
        }
        if network::matched_connection(&saved.identity).ok().flatten() != Some(connection) {
            controller.network_changed(Instant::now());
            display(&status, controller.message());
            last_connection = None;
            continue;
        }
        let outcome = match action {
            Action::Login => match windows_ui::read_password(&saved.username) {
                Ok(Some(mut password)) => {
                    let result = portal.login(&saved.username, &password).map(Some);
                    erase_password(&mut password);
                    result
                }
                _ => {
                    controller.credentials_deleted();
                    display(&status, controller.message());
                    continue;
                }
            },
            Action::Redial => portal.redial().map(|()| None),
            _ => unreachable!(),
        };
        if matches!(outcome, Err(PortalError::NetworkChanged)) {
            last_connection = None;
        }
        controller.on_action_result(Instant::now(), action, outcome);
        display(&status, controller.message());
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
    thread::spawn(move || worker(rx, worker_status, config, worker_generation, worker_gate));
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
    let _ = windows_ui::run(initial, adapters, tx, status, generation, gate);
}
