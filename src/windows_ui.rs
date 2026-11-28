#[cfg(windows)]
#[path = "ui_theme.rs"]
mod theme;

#[cfg(any(windows, test))]
#[path = "update_notice.rs"]
mod update_notice;

#[cfg(test)]
mod tests {
    #[test]
    fn tray_callbacks_preserve_legacy_and_accept_keyboard_activation() {
        use super::{tray_action, TrayAction};
        assert_eq!(tray_action(1, 0x203), Some(TrayAction::Open));
        assert_eq!(
            tray_action(1, 0x205),
            Some(TrayAction::Menu { use_icon: false })
        );
        assert_eq!(tray_action(0, 0x1_0401), Some(TrayAction::Open));
        assert_eq!(
            tray_action(0, 0x1_007b),
            Some(TrayAction::Menu { use_icon: true })
        );
        assert_eq!(tray_action(0, 0x2_0401), None);
        assert_eq!(tray_action(2, 0x205), None);
        assert_eq!(tray_action(0, 0x1_0205), None);
    }
    #[test]
    fn panel_width_fits_a_1280_pixel_work_area_at_200_percent() {
        let panel = super::PanelWidth::fit(1280, 56, 192);
        assert!(super::scale_for_dpi(panel.width, 192) + 56 <= 1280);
        assert!(panel.x(192) > panel.sidebar);
        assert!(panel.x(736) < panel.width);
        assert!(panel.content_span(240) >= 150);
    }

    #[test]
    fn full_width_keeps_normal_window_geometry() {
        let panel = super::PanelWidth::fit(1920, 24, 144);
        assert_eq!(panel.width, 760);
        for x in [0, 12, 168, 192, 476, 736, 760] {
            assert_eq!(panel.x(x), x);
        }
    }
    #[test]
    fn tray_menu_summarizes_networks_without_expanding_to_full_status_text() {
        assert_eq!(
            super::connection_heading(
                "Wi-Fi：校园网在线\r\n以太网：校园网在线\r\n系统选路：校园网 IPv4 首选 WHUT-ISP"
            ),
            "校园网已连接"
        );
        assert_eq!(
            super::connection_heading(
                "Wi-Fi：校园网在线\r\n以太网：校园网络不匹配或不可用，已暂停"
            ),
            "部分网络已连接"
        );
        assert_eq!(
            super::connection_heading(
                "Wi-Fi：需要人工处理验证码，请打开认证页面\r\n以太网：校园网在线\r\n系统选路：以太网兜底"
            ),
            "部分网络已连接"
        );
    }

    #[test]
    fn status_badge_only_calls_the_exact_online_state_online() {
        assert_eq!(
            super::connection_tone("校园网在线"),
            super::ConnectionTone::Online
        );
        for status in [
            "",
            "状态不可用",
            "尚未在线",
            "校园网在线状态未知",
            "上次校园网在线，当前已失效",
            "校园认证成功，等待检查运营商拨号",
        ] {
            assert_ne!(
                super::connection_tone(status),
                super::ConnectionTone::Online,
                "{status}"
            );
        }
    }
    #[test]
    fn status_badge_highlights_manual_intervention_without_claiming_online() {
        for status in [
            "自动操作已暂停 30 分钟",
            "需要人工处理",
            "门户提示密码有误，请检查并重新保存设置",
            "门户未接受登录，请打开认证页面检查",
        ] {
            assert_eq!(
                super::connection_tone(status),
                super::ConnectionTone::Attention,
                "{status}"
            );
        }
        assert_eq!(
            super::connection_tone("正在检查网络"),
            super::ConnectionTone::Pending
        );
    }
    #[test]
    fn dual_connection_status_requires_both_networks_online() {
        assert_eq!(
            super::connection_tone("Wi-Fi：校园网在线\r\n以太网：校园网在线"),
            super::ConnectionTone::Online
        );
        assert_eq!(
            super::connection_tone("Wi-Fi：校园网在线\r\n以太网：正在等待认证或运营商拨号结果"),
            super::ConnectionTone::Partial
        );
        assert_eq!(
            super::connection_tone(
                "Wi-Fi：校园网在线\r\n以太网：门户未接受登录，请打开认证页面检查"
            ),
            super::ConnectionTone::Partial
        );
    }
    #[test]
    fn dual_connection_notification_only_reports_new_actionable_statuses() {
        let old =
            "Wi-Fi：需要人工处理验证码，请打开认证页面\r\n以太网：正在等待认证或运营商拨号结果";
        let ethernet_online = "Wi-Fi：需要人工处理验证码，请打开认证页面\r\n以太网：校园网在线";
        let ethernet_error = "Wi-Fi：需要人工处理验证码，请打开认证页面\r\n以太网：门户未接受登录，请打开认证页面检查";
        assert!(!super::should_notify(old, ethernet_online));
        assert!(super::should_notify(old, ethernet_error));
    }
    #[test]
    fn route_policy_status_does_not_change_the_campus_connection_tone() {
        let online = "Wi-Fi：校园网在线\r\n以太网：校园网在线";
        let offline = "Wi-Fi：校园网在线\r\n以太网：正在等待认证或运营商拨号结果";
        let route = "系统选路：需要以管理员身份运行才能自动切换";
        assert_eq!(
            super::connection_tone(&format!("{online}\r\n{route}")),
            super::ConnectionTone::Online,
        );
        assert_eq!(
            super::connection_tone(&format!("{offline}\r\n{route}")),
            super::ConnectionTone::Partial,
        );
        assert_eq!(
            super::connection_tone(route),
            super::ConnectionTone::Pending
        );
    }
    #[test]
    fn route_policy_status_never_triggers_an_authentication_notification() {
        let campus = "Wi-Fi：校园网在线\r\n以太网：校园网在线";
        let before = format!("{campus}\r\n系统选路：首选 WHUT-ISP");
        let route_issue = format!("{campus}\r\n系统选路：自动操作已暂停，请人工处理");
        assert!(!super::should_notify(&before, &route_issue));
        assert!(super::should_notify(
            &before,
            "Wi-Fi：需要人工处理验证码，请打开认证页面\r\n以太网：校园网在线\r\n系统选路：以太网兜底"
        ));
    }
    #[test]
    fn quit_invalidates_inflight_before_worker_handles_event() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Mutex,
        };
        let generation = AtomicU64::new(4);
        let gate = Mutex::new(());
        let (tx, rx) = mpsc::channel();
        super::notify_quit(&generation, &gate, &tx);
        assert_eq!(generation.load(Ordering::SeqCst), 5);
        assert!(matches!(rx.try_recv(), Ok(super::Event::Quit)));
    }
    #[test]
    fn quit_still_invalidates_and_notifies_when_gate_is_poisoned() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Arc, Mutex,
        };
        let generation = AtomicU64::new(4);
        let gate = Arc::new(Mutex::new(()));
        let poison = Arc::clone(&gate);
        let _ = std::thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("poison test gate");
        })
        .join();
        let (tx, rx) = mpsc::channel();
        super::notify_quit(&generation, &gate, &tx);
        assert_eq!(generation.load(Ordering::SeqCst), 5);
        assert!(gate.is_poisoned());
        assert!(matches!(rx.try_recv(), Ok(super::Event::Quit)));
    }
    #[test]
    fn power_transition_invalidates_generation_before_worker_handles_event() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Mutex,
        };
        let generation = AtomicU64::new(4);
        let gate = Mutex::new(());
        let (tx, rx) = mpsc::channel();
        super::notify_network_change(&generation, &gate, &tx).unwrap();
        assert_eq!(generation.load(Ordering::SeqCst), 5);
        assert!(matches!(rx.try_recv(), Ok(super::Event::NetworkChanged(5))));
        super::notify_network_change(&generation, &gate, &tx).unwrap();
        assert_eq!(generation.load(Ordering::SeqCst), 6);
        assert!(matches!(rx.try_recv(), Ok(super::Event::NetworkChanged(6))));
    }

    #[test]
    fn poisoned_power_transition_gate_does_not_queue_a_resume() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Arc, Mutex,
        };
        let generation = AtomicU64::new(4);
        let gate = Arc::new(Mutex::new(()));
        let poison = Arc::clone(&gate);
        let _ = std::thread::spawn(move || {
            let _held = poison.lock().unwrap();
            panic!("poison test gate");
        })
        .join();
        let (tx, rx) = mpsc::channel();
        assert!(super::notify_network_change(&generation, &gate, &tx).is_err());
        assert_eq!(generation.load(Ordering::SeqCst), 4);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn layout_scales_with_system_dpi() {
        assert_eq!(super::scale_for_dpi(100, 96), 100);
        assert_eq!(super::scale_for_dpi(100, 120), 125);
        assert_eq!(super::scale_for_dpi(100, 144), 150);
        assert_eq!(super::scale_for_dpi(1, 144), 2);
    }
    #[test]
    fn requires_explicit_consent_before_saving() {
        assert!(super::settings_allowed("adapter-1", "", "alice", true));
        assert!(super::settings_allowed(
            "adapter-1",
            "adapter-2",
            "alice",
            true
        ));
        assert!(!super::settings_allowed(
            "adapter-1",
            "adapter-1",
            "alice",
            true
        ));
        assert!(!super::settings_allowed("adapter-1", "", "alice", false));
        assert!(!super::settings_allowed("", "adapter-2", "alice", true));
        assert!(!super::settings_allowed("adapter-1", "", "", true));
    }
    #[test]
    fn adapter_options_keep_both_saved_networks_when_they_disconnect() {
        let settings = super::Settings {
            adapter_id: "known-wifi".into(),
            secondary_adapter_id: "known-ethernet".into(),
            ..Default::default()
        };
        let connected = vec![super::Adapter {
            id: "known-wifi".into(),
            name: "Wi-Fi / WHUT-ISP".into(),
        }];
        let adapters = super::adapter_options(connected, &settings, ["", ""]);
        assert_eq!(adapters.len(), 2);
        assert_eq!(adapters[0].name, "Wi-Fi / WHUT-ISP");
        assert_eq!(adapters[1].id, "known-ethernet");
        assert!(adapters[1].name.contains("当前未连接"));
        let disconnected = super::adapter_options(Vec::new(), &settings, ["", ""]);
        assert_eq!(disconnected.len(), 2);
        assert_eq!(disconnected[0].id, "known-wifi");
        assert_eq!(disconnected[1].id, "known-ethernet");
    }

    #[test]
    fn overview_labels_follow_the_current_adapter_snapshot() {
        let settings = super::Settings {
            adapter_id: "wifi-id".into(),
            secondary_adapter_id: "ethernet-id".into(),
            ..Default::default()
        };
        let wifi = super::Adapter {
            id: "wifi-id".into(),
            name: "Wi-Fi / WHUT-ISP".into(),
        };
        let ethernet = super::Adapter {
            id: "ethernet-id".into(),
            name: "以太网".into(),
        };
        let (only_wifi, _) = super::overview_network_text(&settings, std::slice::from_ref(&wifi));
        assert_eq!(
            only_wifi,
            "主网络　Wi-Fi / WHUT-ISP\r\n第二个网络　已保存，当前不可用"
        );
        let (both, _) = super::overview_network_text(&settings, &[wifi, ethernet]);
        assert_eq!(both, "主网络　Wi-Fi / WHUT-ISP\r\n第二个网络　以太网");
    }

    #[test]
    fn adapter_options_do_not_turn_a_disconnected_draft_into_no_secondary() {
        let settings = super::Settings {
            adapter_id: "known-wifi".into(),
            secondary_adapter_id: "known-ethernet".into(),
            ..Default::default()
        };
        let adapters = super::adapter_options(Vec::new(), &settings, ["known-wifi", "draft"]);
        assert!(adapters
            .iter()
            .any(|adapter| adapter.id == "known-ethernet"));
        assert!(adapters
            .iter()
            .any(|adapter| adapter.id == "draft" && adapter.name.contains("当前未连接")));
    }

    #[test]
    fn installed_autostart_matches_only_the_current_executable() {
        let bytes = |value: &str| {
            value
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        };
        let exe = r"C:\Users\Example\AppData\Local\Programs\CampusNetworkGuardian\campus-network-guardian.exe";
        assert!(super::autostart_value_matches(
            1,
            &bytes(&format!("\"{exe}\"")),
            exe
        ));
        assert!(super::autostart_value_matches(
            1,
            &bytes(&exe.to_ascii_uppercase()),
            exe
        ));
        for command in [
            format!("\"{exe}\" --unexpected"),
            format!("{exe}.other"),
            r"C:\OldCopy\campus-network-guardian.exe".into(),
        ] {
            assert!(!super::autostart_value_matches(1, &bytes(&command), exe));
        }
        assert!(!super::autostart_value_matches(2, &bytes(exe), exe));
        assert!(!super::autostart_value_matches(1, &[1, 2, 3], exe));
        assert!(!super::autostart_value_matches(1, &[0x00, 0xd8, 0, 0], exe));
    }

    #[test]
    fn tooltip_never_splits_a_surrogate_pair() {
        let mut buffer = [0; 4];
        super::copy_utf16("a😀b", &mut buffer);
        assert_eq!(buffer, [97, 0xd83d, 0xde00, 0]);
    }
    #[test]
    fn notification_fires_only_for_a_new_actionable_status() {
        assert!(super::should_notify("在线", "需要人工处理"));
        assert!(super::should_notify("在线", "自动操作已暂停 30 分钟"));
        assert!(!super::should_notify("需要人工处理", "需要人工处理"));
        assert!(!super::should_notify("断线", "在线"));
    }
    #[test]
    fn authentication_failures_notify_once_but_transient_statuses_do_not() {
        for status in [
            "登录预检提示密码有误，请检查并重新保存设置",
            "登录预检提示账号异常，请打开认证页面检查",
            "登录预检提示认证参数异常，请打开认证页面检查",
            "登录预检未通过，请打开认证页面检查",
            "门户提示密码有误，请检查并重新保存设置",
            "门户提示账号异常，请打开认证页面检查",
            "门户提示认证参数异常，请打开认证页面检查",
            "门户未接受登录，请打开认证页面检查",
            "校园认证参数无法识别，自动登录已暂停，请检查网络",
            "登录响应无法识别，自动登录已暂停，请检查并重新保存设置",
        ] {
            assert!(super::should_notify("校园网在线", status), "{status}");
            assert!(!super::should_notify(status, status), "{status}");
        }
        for status in [
            "校园网在线",
            "校园认证成功，等待检查运营商拨号",
            "认证门户不可达，将延迟检查",
            "门户响应异常，将自动重试",
            "请求结果未知，先观察门户状态",
            "连续失败，将在 5 分钟后自动重试",
        ] {
            assert!(!super::should_notify("正在启动", status), "{status}");
        }
    }
    #[test]
    fn credential_username_comparison_stops_at_either_terminator() {
        let stored = [u16::from(b'a'), u16::from(b'b'), 0];
        unsafe {
            assert!(super::matches_terminated_username(stored.as_ptr(), "ab"));
            assert!(!super::matches_terminated_username(
                stored.as_ptr(),
                "abcdef"
            ));
            assert!(!super::matches_terminated_username(stored.as_ptr(), "a"));
            assert!(!super::matches_terminated_username(stored.as_ptr(), "ac"));
            assert!(!super::matches_terminated_username(std::ptr::null(), "ab"));
            assert!(!super::matches_terminated_username(
                stored.as_ptr(),
                "ab\0extra"
            ));
        }
    }
    #[test]
    fn settings_transaction_restores_system_values_when_commit_fails() {
        let calls = std::cell::RefCell::new(Vec::new());
        let result = super::settings_transaction(
            || {
                calls.borrow_mut().push("apply");
                Ok(())
            },
            || {
                calls.borrow_mut().push("commit");
                Err("config failed".into())
            },
            || {
                calls.borrow_mut().push("restore");
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(*calls.borrow(), vec!["apply", "commit", "restore"]);
    }
    #[test]
    fn settings_failure_explains_how_to_resume() {
        let message = super::settings_failure_message("保存失败");
        assert!(message.contains("保存失败"));
        assert!(message.contains("自动操作已暂停，请重新保存设置后恢复"));
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionTone {
    Online,
    Partial,
    Attention,
    Pending,
}
fn connection_tone(status: &str) -> ConnectionTone {
    let (online, total) = campus_status_lines(status)
        .map(status_message)
        .fold((0, 0), |(online, total), message| {
            (online + usize::from(message == "校园网在线"), total + 1)
        });
    if online != 0 && online == total {
        ConnectionTone::Online
    } else if online != 0 {
        ConnectionTone::Partial
    } else if should_notify("", status)
        || campus_status_lines(status).any(|line| {
            line.contains("暂停")
                || line.contains("人工处理")
                || line.contains("密码有误")
                || line.contains("账号异常")
        })
    {
        ConnectionTone::Attention
    } else {
        ConnectionTone::Pending
    }
}
fn campus_status_lines(status: &str) -> impl Iterator<Item = &str> {
    status
        .lines()
        .filter(|line| !line.starts_with("系统选路："))
}
fn connection_heading(status: &str) -> &'static str {
    match connection_tone(status) {
        ConnectionTone::Online => "校园网已连接",
        ConnectionTone::Partial => "部分网络已连接",
        ConnectionTone::Attention => "连接需要关注",
        ConnectionTone::Pending => "正在确认连接",
    }
}
fn status_message(line: &str) -> &str {
    line.split_once('：').map_or(line, |(_, message)| message)
}
fn settings_failure_message(error: &str) -> String {
    format!("{error}\n自动操作已暂停，请重新保存设置后恢复。")
}
fn settings_transaction(
    apply: impl FnOnce() -> Result<(), String>,
    commit: impl FnOnce() -> Result<(), String>,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match apply().and_then(|()| commit()) {
        Ok(()) => Ok(()),
        Err(error) => match restore() {
            Ok(()) => Err(error),
            Err(restore_error) => Err(format!("{error}；恢复原设置失败：{restore_error}")),
        },
    }
}
unsafe fn matches_terminated_username(stored: *const u16, expected: &str) -> bool {
    if stored.is_null() {
        return false;
    }
    let mut index = 0;
    for unit in expected.encode_utf16() {
        let actual = *stored.add(index);
        if unit == 0 || actual == 0 || actual != unit {
            return false;
        }
        index += 1;
    }
    *stored.add(index) == 0
}
fn should_notify(old: &str, new: &str) -> bool {
    campus_status_lines(new).any(|line| {
        let message = status_message(line);
        !old.lines().any(|previous| previous == line)
            && (message.contains("人工处理")
                || message.contains("自动操作已暂停")
                || message.contains("自动登录已暂停")
                || matches!(
                    message,
                    "登录预检提示密码有误，请检查并重新保存设置"
                        | "登录预检提示账号异常，请打开认证页面检查"
                        | "登录预检提示认证参数异常，请打开认证页面检查"
                        | "登录预检未通过，请打开认证页面检查"
                        | "门户提示密码有误，请检查并重新保存设置"
                        | "门户提示账号异常，请打开认证页面检查"
                        | "门户提示认证参数异常，请打开认证页面检查"
                        | "门户未接受登录，请打开认证页面检查"
                ))
    })
}
fn copy_utf16(text: &str, output: &mut [u16]) {
    output.fill(0);
    let mut position = 0;
    for character in text.chars() {
        let mut units = [0; 2];
        let encoded = character.encode_utf16(&mut units);
        if position + encoded.len() >= output.len() {
            break;
        }
        output[position..position + encoded.len()].copy_from_slice(encoded);
        position += encoded.len();
    }
}
fn settings_allowed(adapter: &str, secondary: &str, username: &str, consent: bool) -> bool {
    !adapter.trim().is_empty() && adapter != secondary && !username.trim().is_empty() && consent
}
fn autostart_value_matches(kind: u32, value: &[u8], exe: &str) -> bool {
    if kind != 1 || value.len() % 2 != 0 {
        return false;
    }
    let units: Vec<u16> = value
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let Ok(command) = String::from_utf16(&units) else {
        return false;
    };
    let command = command.trim_end_matches('\0').trim();
    command.eq_ignore_ascii_case(exe) || command.eq_ignore_ascii_case(&format!("\"{exe}\""))
}
fn scale_for_dpi(value: i32, dpi: u32) -> i32 {
    ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32
}

#[derive(Clone, Copy)]
struct PanelWidth {
    width: i32,
    sidebar: i32,
}

impl PanelWidth {
    const STANDARD: Self = Self {
        width: 760,
        sidebar: theme::SIDEBAR_WIDTH,
    };

    fn fit(work_width: i32, chrome_width: i32, dpi: u32) -> Self {
        let width = ((i64::from((work_width - chrome_width).max(1)) * 96) / i64::from(dpi)) as i32;
        let width = width.clamp(1, Self::STANDARD.width);
        let sidebar = if width >= 680 {
            168
        } else if width >= 540 {
            144
        } else {
            128.min(width / 3)
        };
        Self { width, sidebar }
    }

    fn x(self, x: i32) -> i32 {
        if x <= Self::STANDARD.sidebar {
            x * self.sidebar / Self::STANDARD.sidebar
        } else {
            self.sidebar + self.content_span(x - Self::STANDARD.sidebar)
        }
    }

    fn content_span(self, width: i32) -> i32 {
        width * (self.width - self.sidebar) / (Self::STANDARD.width - Self::STANDARD.sidebar)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Adapter {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub adapter_id: String,
    pub secondary_adapter_id: String,
    pub username: String,
    pub http_acknowledged: bool,
    pub autostart: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    Current,
    Installing,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub source: String,
    pub changelog: String,
    pub progress: String,
}

#[cfg(windows)]
const APPLICATION_VERSION: &str = match option_env!("CARGO_PKG_VERSION") {
    Some(version) => version,
    None => "0.7.0",
};

pub struct UiUpdates {
    pub request: Box<dyn Fn()>,
    pub status: Box<dyn Fn() -> UpdateState>,
    pub info: Box<dyn Fn() -> UpdateInfo>,
}

pub struct UiAnalytics {
    pub enabled: Box<dyn Fn() -> bool>,
    pub set_enabled: Box<dyn Fn(bool) -> Result<(), String>>,
}

pub struct UiDiagnostics {
    pub summary: Box<dyn Fn() -> String>,
    pub report: Box<dyn Fn() -> String>,
    pub check_external: Box<dyn Fn()>,
}

const DISCONNECTED_LABEL: &str = "（当前未连接）";
const TRAY_ICON_ID: u32 = 1;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
#[derive(Debug, PartialEq, Eq)]
enum TrayAction {
    Open,
    Menu { use_icon: bool },
}
fn tray_action(icon_or_anchor: usize, packed: isize) -> Option<TrayAction> {
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_CONTEXTMENU: u32 = 0x007b;
    const NIN_KEYSELECT: u32 = 0x0401;
    let packed = packed as u32;
    let icon_id = packed >> 16;
    match (icon_id, packed & 0xffff) {
        (TRAY_ICON_ID, WM_CONTEXTMENU) => Some(TrayAction::Menu { use_icon: true }),
        (TRAY_ICON_ID, NIN_KEYSELECT | WM_LBUTTONDBLCLK) => Some(TrayAction::Open),
        (0, WM_LBUTTONDBLCLK) if icon_or_anchor == TRAY_ICON_ID as usize => Some(TrayAction::Open),
        (0, WM_RBUTTONUP) if icon_or_anchor == TRAY_ICON_ID as usize => {
            Some(TrayAction::Menu { use_icon: false })
        }
        _ => None,
    }
}
fn overview_network_text(settings: &Settings, adapters: &[Adapter]) -> (String, String) {
    const UNAVAILABLE: &str = "已保存，当前不可用";
    let name = |id: &str| {
        adapters
            .iter()
            .find(|adapter| adapter.id == id)
            .map(|adapter| adapter.name.as_str())
    };
    let primary = name(&settings.adapter_id).unwrap_or(if settings.adapter_id.is_empty() {
        "尚未选择校园网络"
    } else {
        UNAVAILABLE
    });
    if settings.secondary_adapter_id.is_empty() {
        primary
            .rsplit_once(" / 网关 ")
            .map(|(network, gateway)| (network.to_owned(), format!("网关 {gateway}")))
            .unwrap_or_else(|| (primary.into(), "仅在匹配所选网络时执行认证".into()))
    } else {
        let secondary = name(&settings.secondary_adapter_id).unwrap_or(UNAVAILABLE);
        (
            format!("主网络　{primary}\r\n第二个网络　{secondary}"),
            "仅在匹配所选网络时执行认证".into(),
        )
    }
}
fn adapter_options(
    mut connected: Vec<Adapter>,
    saved: &Settings,
    draft: [&str; 2],
) -> Vec<Adapter> {
    for (id, label) in [
        (saved.adapter_id.as_str(), "已保存的主网络"),
        (saved.secondary_adapter_id.as_str(), "已保存的第二个网络"),
        (draft[0], "待保存的主网络"),
        (draft[1], "待保存的第二个网络"),
    ] {
        if !id.is_empty() && !connected.iter().any(|adapter| adapter.id == id) {
            connected.push(Adapter {
                id: id.into(),
                name: format!("{label}{DISCONNECTED_LABEL}"),
            });
        }
    }
    connected
}
#[derive(Clone, Debug)]
pub enum Event {
    CheckNow,
    SettingsSaved(Settings, std::sync::mpsc::Sender<Result<(), String>>),
    SettingsFailed,
    CredentialsDeleted,
    NetworkChanged(u64),
    Quit,
}

fn notify_network_change(
    generation: &std::sync::atomic::AtomicU64,
    gate: &std::sync::Mutex<()>,
    tx: &std::sync::mpsc::Sender<Event>,
) -> Result<(), String> {
    let _guard = gate.lock().map_err(|_| "网络操作锁异常".to_string())?;
    let updated = generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        .wrapping_add(1);
    tx.send(Event::NetworkChanged(updated))
        .map_err(|_| "控制器不可用".to_string())
}

fn notify_quit(
    generation: &std::sync::atomic::AtomicU64,
    gate: &std::sync::Mutex<()>,
    tx: &std::sync::mpsc::Sender<Event>,
) {
    // Even a poisoned gate is held while invalidating; POST rejects poisoned gates.
    let _guard = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _ = tx.send(Event::Quit);
}

#[cfg(windows)]
mod win {
    use super::theme;
    use super::{
        adapter_options, autostart_value_matches, connection_heading, copy_utf16,
        matches_terminated_username, notify_network_change, notify_quit, overview_network_text,
        scale_for_dpi, settings_allowed, settings_failure_message, settings_transaction,
        should_notify, Adapter, Event, PanelWidth, Settings, UiAnalytics, UiDiagnostics, UiUpdates,
        UpdateState,
    };
    use super::{tray_action, TrayAction, TRAY_ICON_ID, WM_LBUTTONDBLCLK};
    use std::{
        cell::{Cell, RefCell},
        ffi::c_void,
        mem::{size_of, zeroed},
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc::{self, Sender},
            Arc, Mutex,
        },
        time::{Duration, Instant},
    };
    type Handle = isize;
    const TARGET: &str = "CampusNetworkGuardian/Portal";
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const APP_NAME: &str = "CampusNetworkGuardian";
    const PORTAL: &str = "http://172.30.21.100/";
    const AUTHOR_URL: &str = "https://github.com/Liunian520";
    const TRAY: u32 = 0x8001;
    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIM_SETFOCUS: u32 = 3;
    const NIM_SETVERSION: u32 = 4;
    const NIF_MESSAGE: u32 = 0x01;
    const NIF_ICON: u32 = 0x02;
    const NIF_TIP: u32 = 0x04;
    const NIF_INFO: u32 = 0x10;
    const NIF_SHOWTIP: u32 = 0x80;
    const NOTIFYICON_VERSION_4: u32 = 4;
    const WM_DESTROY: u32 = 2;
    const WM_CLOSE: u32 = 0x10;
    const WM_SETTINGCHANGE: u32 = 0x1a;
    const WM_COMMAND: u32 = 0x111;
    const WM_TIMER: u32 = 0x113;
    const WM_POWERBROADCAST: u32 = 0x218;
    const WM_CTLCOLORSTATIC: u32 = 0x138;
    const WM_CTLCOLORBTN: u32 = 0x135;
    const WM_CTLCOLOREDIT: u32 = 0x133;
    const WM_CTLCOLORLISTBOX: u32 = 0x134;
    const WM_ERASEBKGND: u32 = 0x14;
    const WM_DRAWITEM: u32 = 0x2b;
    const WM_VSCROLL: u32 = 0x115;
    const WM_MOUSEWHEEL: u32 = 0x20a;
    const WM_SIZE: u32 = 5;
    const C_ADAPTER: usize = 101;
    const C_USER: usize = 102;
    const C_PASS: usize = 103;
    const C_CONSENT: usize = 104;
    const C_START: usize = 105;
    const C_SAVE: usize = 106;
    const C_DELETE: usize = 107;
    const C_TITLE: usize = 108;
    const C_SUBTITLE: usize = 109;
    const C_STATUS: usize = 110;
    const C_HINT_NETWORK: usize = 111;
    const C_HINT_PASSWORD: usize = 112;
    const C_STATUS_LABEL: usize = 113;
    const C_SECTION: usize = 114;
    const C_FOOTER: usize = 115;
    const C_STATUS_HEAD: usize = 116;
    const C_NETWORK_VALUE: usize = 117;
    const C_NETWORK_LABEL: usize = 119;
    const C_OVERVIEW_HINT: usize = 121;
    const C_NETWORK_DESC: usize = 122;
    const C_FORM_SUBTITLE: usize = 124;
    const C_FORM_NETWORK: usize = 125;
    const C_FORM_ACCOUNT: usize = 126;
    const C_LABEL_USER: usize = 127;
    const C_LABEL_PASS: usize = 128;
    const C_START_HINT: usize = 129;
    const C_SETTINGS_TITLE: usize = 130;
    const C_FORM_FEEDBACK: usize = 131;
    const C_FORM_AUTOMATION: usize = 132;
    const C_HTTP_HINT: usize = 133;
    const C_ADAPTER_DETAIL: usize = 134;
    const C_SECONDARY_ADAPTER: usize = 135;
    const C_LABEL_ADAPTER: usize = 136;
    const C_LABEL_SECONDARY_ADAPTER: usize = 137;
    const C_ANALYTICS: usize = 138;
    const C_ANALYTICS_DISCLOSURE: usize = 139;
    const C_DIAGNOSTICS_TITLE: usize = 140;
    const C_DIAGNOSTICS_SUMMARY: usize = 141;
    const C_DIAGNOSTICS_HISTORY: usize = 142;
    const C_DIAGNOSTICS_COPY: usize = 143;
    const C_DIAGNOSTICS_REPORT: usize = 144;
    const C_DIAGNOSTICS_FEEDBACK: usize = 145;
    const C_NAV_OVERVIEW: usize = 301;
    const C_NAV_SETTINGS: usize = 302;
    const C_AUTHOR: usize = 303;
    const C_ABOUT: usize = 304;
    const C_UPDATE: usize = 305;
    const C_PAGE_SCROLL: usize = 306;
    const M_CHECK: usize = 201;
    const M_SETTINGS: usize = 202;
    const M_OPEN: usize = 203;
    const M_QUIT: usize = 204;
    const M_TRAY_PANEL: usize = 205;
    const M_ANALYTICS: usize = 206;
    const M_CHECK_EXTERNAL: usize = 207;
    const BM_GETCHECK: u32 = 0xf0;
    const BM_SETCHECK: u32 = 0xf1;
    const WS_CHILD: u32 = 0x40000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_TABSTOP: u32 = 0x10000;
    const SB_CTL: i32 = 2;
    const OBJID_CLIENT: i32 = -4;
    const SCROLL_REVEAL_TIMER: usize = 2;
    const SCROLL_REVEAL_MS: u32 = 850;
    const CONTENT_HEIGHT: i32 = 600;
    const COMPACT_OVERVIEW_HEIGHT: i32 = 360;
    const COMPACT_OVERVIEW_SHIFT: i32 = 32;
    const DIAGNOSTICS_TOP: i32 = 320;
    const RECOVERY_REPORT_HEIGHT: i32 = 156;
    const NETWORK_FIELD_ROW_HEIGHT: i32 = 24;
    const ADAPTER_COMBO_WIDTH: i32 = 220;
    const ANALYTICS_TOP: i32 = 596;
    const ANALYTICS_DISCLOSURE_TOP: i32 = 646;
    const ANALYTICS_DISCLOSURE: &str = "允许后，每天最多发送一次应用活跃事件，包含随机安装标识和应用版本。事件不包含姓名、校园网账号、密码、网卡或 Wi-Fi 信息。取消勾选会停止新上报并清除本机安装标识。已经发送的请求可能完成。";
    const HKEY_CURRENT_USER: Handle = 0x80000001u32 as i32 as isize;
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct ScrollInfo {
        size: u32,
        mask: u32,
        min: i32,
        max: i32,
        page: u32,
        position: i32,
        track_position: i32,
    }
    #[repr(C)]
    struct ScrollBarInfo {
        size: u32,
        bounds: Rect,
        arrow_size: i32,
        thumb_top: i32,
        thumb_bottom: i32,
        reserved: i32,
        state: [u32; 6],
    }
    #[repr(C)]
    struct DrawItem {
        control_type: u32,
        control_id: u32,
        item_id: u32,
        action: u32,
        state: u32,
        item: Handle,
        dc: Handle,
        rect: Rect,
        data: usize,
    }
    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }
    #[repr(C)]
    struct TrackMouse {
        size: u32,
        flags: u32,
        hwnd: Handle,
        hover_time: u32,
    }
    #[repr(C)]
    struct Paint {
        dc: Handle,
        erase: i32,
        rect: Rect,
        restore: i32,
        update: i32,
        reserved: [u8; 32],
    }
    struct ControlLayout {
        hwnd: Handle,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        page: u8,
    }
    thread_local! { static LAYOUT: RefCell<Vec<ControlLayout>> = const { RefCell::new(Vec::new()) }; }
    #[repr(C)]
    struct Msg {
        hwnd: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
        private: u32,
    }
    #[repr(C)]
    struct WndClass {
        style: u32,
        proc: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
        cls_extra: i32,
        wnd_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu: *const u16,
        class: *const u16,
    }
    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    #[repr(C)]
    struct Notify {
        size: u32,
        hwnd: Handle,
        id: u32,
        flags: u32,
        callback: u32,
        icon: Handle,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        version_or_timeout: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: Guid,
        balloon: Handle,
    }
    #[repr(C)]
    struct NotifyIconIdentifier {
        size: u32,
        hwnd: Handle,
        id: u32,
        guid: Guid,
    }
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct Credential {
        flags: u32,
        kind: u32,
        target: *mut u16,
        comment: *mut u16,
        written: FileTime,
        blob_size: u32,
        blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut c_void,
        alias: *mut u16,
        username: *mut u16,
    }
    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(c: *const WndClass) -> u16;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn ChangeWindowMessageFilterEx(
            h: Handle,
            message: u32,
            action: u32,
            filter: *mut c_void,
        ) -> i32;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *const c_void,
        ) -> Handle;
        fn DefWindowProcW(h: Handle, m: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(m: *mut Msg, h: Handle, min: u32, max: u32) -> i32;
        fn PostMessageW(h: Handle, message: u32, w: usize, l: isize) -> i32;
        fn IsDialogMessageW(h: Handle, m: *mut Msg) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
        fn ShowWindow(h: Handle, n: i32) -> i32;
        fn IsWindowVisible(h: Handle) -> i32;
        fn SetForegroundWindow(h: Handle) -> i32;
        fn DestroyWindow(h: Handle) -> i32;
        fn PostQuitMessage(code: i32);
        fn GetDlgItem(h: Handle, id: i32) -> Handle;
        fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
        fn SetTimer(h: Handle, id: usize, delay: u32, callback: *const c_void) -> usize;
        fn KillTimer(h: Handle, id: usize) -> i32;
        fn SendMessageW(h: Handle, m: u32, w: usize, l: isize) -> isize;
        fn GetWindowTextLengthW(h: Handle) -> i32;
        fn GetWindowTextW(h: Handle, s: *mut u16, n: i32) -> i32;
        fn SetWindowTextW(h: Handle, s: *const u16) -> i32;
        fn OpenClipboard(h: Handle) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: Handle) -> Handle;
        fn CloseClipboard() -> i32;
        fn MessageBoxW(h: Handle, text: *const u16, title: *const u16, style: u32) -> i32;
        fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
        fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
        fn CreatePopupMenu() -> Handle;
        fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
        fn TrackPopupMenu(
            menu: Handle,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            h: Handle,
            rect: *const c_void,
        ) -> i32;
        fn DestroyMenu(menu: Handle) -> i32;
        fn GetCursorPos(p: *mut Point) -> i32;
        fn GetDlgCtrlID(h: Handle) -> i32;
        fn MoveWindow(h: Handle, x: i32, y: i32, w: i32, height: i32, repaint: i32) -> i32;
        fn InvalidateRect(h: Handle, rect: *const Rect, erase: i32) -> i32;
        fn TrackMouseEvent(event: *mut TrackMouse) -> i32;
        fn BeginPaint(h: Handle, paint: *mut Paint) -> Handle;
        fn EndPaint(h: Handle, paint: *const Paint) -> i32;
        fn SetProcessDPIAware() -> i32;
        fn GetDpiForSystem() -> u32;
        fn GetSystemMetrics(index: i32) -> i32;
        fn SystemParametersInfoW(action: u32, param: u32, data: *mut c_void, flags: u32) -> i32;
        fn GetClientRect(h: Handle, rect: *mut Rect) -> i32;
        fn GetDC(h: Handle) -> Handle;
        fn ReleaseDC(h: Handle, dc: Handle) -> i32;
        fn GetWindowRect(h: Handle, rect: *mut Rect) -> i32;
        fn GetFocus() -> Handle;
        fn GetCapture() -> Handle;
        fn SetFocus(h: Handle) -> Handle;
        fn GetParent(h: Handle) -> Handle;
        fn MapWindowPoints(from: Handle, to: Handle, points: *mut Point, count: u32) -> i32;
        fn GetScrollInfo(h: Handle, bar: i32, info: *mut ScrollInfo) -> i32;
        fn SetScrollInfo(h: Handle, bar: i32, info: *const ScrollInfo, redraw: i32) -> i32;
        fn GetScrollBarInfo(h: Handle, object: i32, info: *mut ScrollBarInfo) -> i32;
        fn ShowScrollBar(h: Handle, bar: i32, show: i32) -> i32;
        fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex: u32) -> i32;
        fn FillRect(dc: Handle, rect: *const Rect, brush: Handle) -> i32;
        fn DrawTextW(dc: Handle, text: *const u16, count: i32, rect: *mut Rect, format: u32)
            -> i32;
        fn DrawIconEx(
            dc: Handle,
            x: i32,
            y: i32,
            icon: Handle,
            cx: i32,
            cy: i32,
            step: u32,
            brush: Handle,
            flags: u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
        fn CreateMutexW(attrs: *const c_void, owner: i32, name: *const u16) -> Handle;
        fn GetLastError() -> u32;
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(memory: Handle) -> *mut c_void;
        fn GlobalUnlock(memory: Handle) -> i32;
        fn GlobalFree(memory: Handle) -> Handle;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(msg: u32, data: *mut Notify) -> i32;
        fn Shell_NotifyIconGetRect(icon: *const NotifyIconIdentifier, rect: *mut Rect) -> i32;
        fn ShellExecuteW(
            h: Handle,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> Handle;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn GetStockObject(index: i32) -> Handle;
        fn CreateFontW(
            height: i32,
            width: i32,
            escapement: i32,
            orientation: i32,
            weight: i32,
            italic: u32,
            underline: u32,
            strikeout: u32,
            charset: u32,
            output_precision: u32,
            clip_precision: u32,
            quality: u32,
            pitch: u32,
            face: *const u16,
        ) -> Handle;
        fn DeleteObject(object: Handle) -> i32;
        fn SetTextColor(dc: Handle, color: u32) -> u32;
        fn SetBkColor(dc: Handle, color: u32) -> u32;
        fn SetBkMode(dc: Handle, mode: i32) -> i32;
        fn CreateSolidBrush(color: u32) -> Handle;
        fn CreatePen(style: i32, width: i32, color: u32) -> Handle;
        fn SelectObject(dc: Handle, object: Handle) -> Handle;
        fn RoundRect(
            dc: Handle,
            left: i32,
            top: i32,
            right: i32,
            bottom: i32,
            width: i32,
            height: i32,
        ) -> i32;
        fn Ellipse(dc: Handle, left: i32, top: i32, right: i32, bottom: i32) -> i32;
        #[link_name = "Arc"]
        fn GdiArc(
            dc: Handle,
            left: i32,
            top: i32,
            right: i32,
            bottom: i32,
            start_x: i32,
            start_y: i32,
            end_x: i32,
            end_y: i32,
        ) -> i32;
        fn MoveToEx(dc: Handle, x: i32, y: i32, previous: *mut Point) -> i32;
        fn LineTo(dc: Handle, x: i32, y: i32) -> i32;
    }
    #[link(name = "uxtheme")]
    extern "system" {
        fn SetWindowTheme(hwnd: Handle, app: *const u16, id: *const u16) -> i32;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(hwnd: Handle, attr: u32, value: *const c_void, size: u32) -> i32;
    }
    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(
            hwnd: Handle,
            proc: Option<
                unsafe extern "system" fn(Handle, u32, usize, isize, usize, usize) -> isize,
            >,
            id: usize,
            data: usize,
        ) -> i32;
        fn DefSubclassProc(hwnd: Handle, message: u32, w: usize, l: isize) -> isize;
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn CredWriteW(c: *const Credential, flags: u32) -> i32;
        fn CredReadW(target: *const u16, kind: u32, flags: u32, c: *mut *mut Credential) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(c: *mut c_void);
        fn RegGetValueW(
            key: Handle,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> u32;
        fn RegOpenKeyExW(
            key: Handle,
            sub: *const u16,
            opt: u32,
            access: u32,
            result: *mut Handle,
        ) -> i32;
        fn RegCreateKeyExW(
            key: Handle,
            sub: *const u16,
            reserved: u32,
            class: *mut u16,
            options: u32,
            access: u32,
            security: *const c_void,
            result: *mut Handle,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: Handle,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            len: u32,
        ) -> i32;
        fn RegQueryValueExW(
            key: Handle,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegDeleteValueW(key: Handle, name: *const u16) -> i32;
        fn RegCloseKey(key: Handle) -> i32;
    }
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }
    fn error(context: &str) -> String {
        format!("{}（Windows 错误 {}）", context, unsafe {
            GetLastError()
        })
    }
    unsafe fn free_credential(p: *mut Credential) {
        let credential = &*p;
        if !credential.blob.is_null() {
            for index in 0..credential.blob_size as usize {
                std::ptr::write_volatile(credential.blob.add(index), 0);
            }
        }
        CredFree(p.cast());
    }
    pub fn single_instance() -> Result<bool, String> {
        unsafe {
            let h = CreateMutexW(
                null(),
                0,
                wide("Local\\CampusNetworkGuardian.Singleton").as_ptr(),
            );
            if h == 0 {
                Err(error("无法创建单实例锁"))
            } else {
                Ok(GetLastError() != 183)
            }
        }
    }
    pub fn show_existing_panel() {
        unsafe {
            let window = FindWindowW(wide("CampusNetworkGuardianWindow").as_ptr(), null());
            if window != 0 {
                PostMessageW(
                    window,
                    TRAY,
                    TRAY_ICON_ID as usize,
                    WM_LBUTTONDBLCLK as isize,
                );
            }
        }
    }
    pub fn autostart_enabled() -> Result<bool, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(snapshot_autostart()?.is_some_and(|(kind, value)| {
            autostart_value_matches(kind, &value, &exe.to_string_lossy())
        }))
    }
    pub fn read_password(username: &str) -> Result<Option<String>, String> {
        unsafe {
            let mut p: *mut Credential = null_mut();
            if CredReadW(wide(TARGET).as_ptr(), 1, 0, &mut p) == 0 {
                return if GetLastError() == 1168 {
                    Ok(None)
                } else {
                    Err(error("读取凭据失败"))
                };
            }
            let c = &*p;
            if !matches_terminated_username(c.username, username) {
                free_credential(p);
                return Ok(None);
            }
            let bytes = if c.blob_size == 0 {
                &[]
            } else if c.blob.is_null() {
                free_credential(p);
                return Err("凭据格式错误".into());
            } else {
                std::slice::from_raw_parts(c.blob, c.blob_size as usize)
            };
            let result = if bytes.len() % 2 != 0 {
                Err("凭据格式错误".into())
            } else {
                let mut units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|x| u16::from_le_bytes([x[0], x[1]]))
                    .collect();
                let result = String::from_utf16(&units)
                    .map(Some)
                    .map_err(|_| "凭据格式错误".into());
                for unit in &mut units {
                    std::ptr::write_volatile(unit, 0);
                }
                result
            };
            free_credential(p);
            result
        }
    }
    pub fn write_password(username: &str, password: &str) -> Result<(), String> {
        unsafe {
            let mut target = wide(TARGET);
            let mut username = wide(username);
            let mut blob: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut c: Credential = zeroed();
            c.kind = 1;
            c.target = target.as_mut_ptr();
            c.username = username.as_mut_ptr();
            c.blob_size = blob.len() as u32;
            c.blob = blob.as_mut_ptr();
            c.persist = 2;
            let ok = CredWriteW(&c, 0);
            let err = if ok == 0 {
                Some(error("保存凭据失败"))
            } else {
                None
            };
            for b in &mut blob {
                std::ptr::write_volatile(b, 0);
            }
            for unit in &mut username {
                std::ptr::write_volatile(unit, 0);
            }
            if let Some(e) = err {
                Err(e)
            } else {
                Ok(())
            }
        }
    }
    pub fn delete_password() -> Result<(), String> {
        unsafe {
            if CredDeleteW(wide(TARGET).as_ptr(), 1, 0) != 0 || GetLastError() == 1168 {
                Ok(())
            } else {
                Err(error("删除凭据失败"))
            }
        }
    }
    pub fn set_autostart(enabled: bool) -> Result<(), String> {
        unsafe {
            let cmd = if enabled {
                Some(wide(&format!(
                    "\"{}\"",
                    std::env::current_exe()
                        .map_err(|e| e.to_string())?
                        .display()
                )))
            } else {
                None
            };
            let mut key = 0;
            let code = if enabled {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    wide(RUN_KEY).as_ptr(),
                    0,
                    null_mut(),
                    0,
                    0x0002,
                    null(),
                    &mut key,
                    null_mut(),
                )
            } else {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    wide(RUN_KEY).as_ptr(),
                    0,
                    0x0002,
                    &mut key,
                )
            };
            if !enabled && code == 2 {
                return Ok(());
            }
            if code != 0 {
                return Err(format!("打开自启动设置失败（Windows 错误 {code}）"));
            }
            let name = wide(APP_NAME);
            let code = if let Some(cmd) = cmd {
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    1,
                    cmd.as_ptr().cast(),
                    (cmd.len() * 2) as u32,
                )
            } else {
                RegDeleteValueW(key, name.as_ptr())
            };
            RegCloseKey(key);
            if code == 0 || (!enabled && code == 2) {
                Ok(())
            } else {
                Err(format!("更新自启动设置失败（Windows 错误 {code}）"))
            }
        }
    }
    // Preserve the exact Run entry so a failed settings save can restore it.
    fn snapshot_autostart() -> Result<Option<(u32, Vec<u8>)>, String> {
        unsafe {
            let mut key = 0;
            let code = RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, 1, &mut key);
            if code == 2 {
                return Ok(None);
            }
            if code != 0 {
                return Err(format!("读取自启动设置失败（Windows 错误 {code}）"));
            }
            let mut kind = 0;
            let mut len = 0;
            let name = wide(APP_NAME);
            let code = RegQueryValueExW(
                key,
                name.as_ptr(),
                null_mut(),
                &mut kind,
                null_mut(),
                &mut len,
            );
            if code == 2 {
                RegCloseKey(key);
                return Ok(None);
            }
            if code != 0 || len > 65536 {
                RegCloseKey(key);
                return Err(format!("读取自启动设置失败（Windows 错误 {code}）"));
            }
            let mut value = vec![0; len as usize];
            let code = RegQueryValueExW(
                key,
                name.as_ptr(),
                null_mut(),
                &mut kind,
                value.as_mut_ptr(),
                &mut len,
            );
            RegCloseKey(key);
            if code != 0 {
                return Err(format!("读取自启动设置失败（Windows 错误 {code}）"));
            }
            value.truncate(len as usize);
            Ok(Some((kind, value)))
        }
    }
    fn restore_autostart(previous: &Option<(u32, Vec<u8>)>) -> Result<(), String> {
        unsafe {
            let mut key = 0;
            let code = RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, 2, &mut key);
            if code == 2 && previous.is_none() {
                return Ok(());
            }
            if code != 0 {
                return Err(format!("恢复自启动失败（Windows 错误 {code}）"));
            }
            let name = wide(APP_NAME);
            let code = match previous {
                Some((kind, data)) => RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    *kind,
                    data.as_ptr(),
                    data.len() as u32,
                ),
                None => RegDeleteValueW(key, name.as_ptr()),
            };
            RegCloseKey(key);
            if code == 0 || (code == 2 && previous.is_none()) {
                Ok(())
            } else {
                Err(format!("恢复自启动失败（Windows 错误 {code}）"))
            }
        }
    }
    struct Secret(String);
    impl Drop for Secret {
        fn drop(&mut self) {
            unsafe {
                for byte in self.0.as_bytes_mut() {
                    std::ptr::write_volatile(byte, 0);
                }
            }
        }
    }
    struct App {
        settings: RefCell<Settings>,
        adapters: RefCell<Vec<Adapter>>,
        adapter_source: Box<dyn Fn() -> Vec<Adapter>>,
        tx: Sender<Event>,
        status: Arc<Mutex<String>>,
        tray: RefCell<Notify>,
        tray_available: Cell<bool>,
        taskbar_created: u32,
        last_status: RefCell<String>,
        update: UiUpdates,
        update_state: Cell<Option<UpdateState>>,
        analytics: UiAnalytics,
        diagnostics: UiDiagnostics,
        history_open: Cell<bool>,
        generation: Arc<AtomicU64>,
        gate: Arc<Mutex<()>>,
    }
    fn invalidate_inflight(app: &App) -> Result<(), String> {
        let _guard = app.gate.lock().map_err(|_| "网络操作锁异常".to_string())?;
        app.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    unsafe fn report_settings_failure(h: Handle, app: &App, error: &str) {
        let _ = app.tx.send(Event::SettingsFailed);
        form_feedback(h, &settings_failure_message(error), C_SAVE);
    }
    unsafe fn report_preflight_failure(h: Handle, app: &App, error: &str) {
        let detail = match invalidate_inflight(app) {
            Ok(()) => error.to_owned(),
            Err(lock_error) => format!("{error}；{lock_error}"),
        };
        report_settings_failure(h, app, &detail);
    }
    // This pointer is initialized and accessed only by the window's UI thread.
    static mut APP: *const App = null();
    // The UI is system-DPI aware; Windows scales the window if moved to a monitor
    // with a different DPI. All values here are confined to the UI thread.
    static mut UI_DPI: u32 = 96;
    static mut PANEL_WIDTH: PanelWidth = PanelWidth::STANDARD;
    static mut UI_FONT: Handle = 0;
    static mut TITLE_FONT: Handle = 0;
    static mut SECTION_FONT: Handle = 0;
    static mut SMALL_FONT: Handle = 0;
    static mut HERO_FONT: Handle = 0;
    static mut BACKGROUND_BRUSH: Handle = 0;
    static mut CARD_BRUSH: Handle = 0;
    static mut HERO_BRUSH: Handle = 0;
    static mut SIDEBAR_BRUSH: Handle = 0;
    static mut APP_ICON: Handle = 0;
    static mut HIGH_CONTRAST: bool = false;
    static mut SCROLL_OFFSET: i32 = 0;
    static mut SCROLL_HOVERED: bool = false;
    static mut SCROLL_ALWAYS_VISIBLE: bool = false;
    static mut SCROLL_REVEALED_UNTIL: Option<Instant> = None;
    static mut ACTIVE_PAGE: u8 = 1;
    static mut BUILD_PAGE: u8 = 0;
    static mut HOVERED_BUTTON: Handle = 0;
    static mut HERO_EXTRA: i32 = 0;
    static mut DIAGNOSTICS_HEIGHT: i32 = 0;
    static mut DIAGNOSTICS_SUMMARY_HEIGHT: i32 = 20;
    static mut NETWORK_TEXT_HEIGHT: i32 = 28;
    static mut NETWORK_DETAIL_HEIGHT: i32 = 20;
    static mut OVERVIEW_HEIGHT: i32 = CONTENT_HEIGHT;
    static mut FORM_SHIFT: i32 = 0;
    static mut FORM_NETWORK_EXTRA: i32 = NETWORK_FIELD_ROW_HEIGHT;
    static mut FORM_ERROR_FIELD: usize = 0;
    #[derive(Clone, Copy)]
    struct FormWrap {
        account: i32,
        consent: i32,
        http_hint: i32,
        startup: i32,
        automation: i32,
        analytics: i32,
    }
    static mut FORM_WRAP: FormWrap = FormWrap {
        account: 0,
        consent: 0,
        http_hint: 0,
        startup: 0,
        automation: 0,
        analytics: 0,
    };
    unsafe fn app() -> Option<&'static App> {
        APP.as_ref()
    }
    unsafe fn text(h: Handle, id: usize) -> String {
        let c = GetDlgItem(h, id as i32);
        let len = GetWindowTextLengthW(c);
        let mut buf = vec![0u16; (len + 1).max(1) as usize];
        let n = GetWindowTextW(c, buf.as_mut_ptr(), buf.len() as i32);
        let result = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        for unit in &mut buf {
            std::ptr::write_volatile(unit, 0);
        }
        result
    }
    unsafe fn alert(h: Handle, msg: &str) {
        MessageBoxW(h, wide(msg).as_ptr(), wide("校园网守护").as_ptr(), 0x30);
    }
    fn application_version() -> &'static str {
        super::APPLICATION_VERSION
    }
    fn notice_snapshot() -> (UpdateState, super::UpdateInfo) {
        unsafe {
            app()
                .map(|app| ((app.update.status)(), (app.update.info)()))
                .unwrap_or((UpdateState::Idle, super::UpdateInfo::default()))
        }
    }
    fn request_notice_check() {
        unsafe {
            if let Some(app) = app() {
                if !matches!(
                    (app.update.status)(),
                    UpdateState::Checking | UpdateState::Installing
                ) {
                    (app.update.request)();
                }
            }
        }
    }
    fn update_caption(state: UpdateState) -> (&'static str, &'static str) {
        match state {
            UpdateState::Idle => ("检查更新", ""),
            UpdateState::Checking => ("正在检查", "正在检查更新"),
            UpdateState::Current => ("已是最新版", "已是最新版本"),
            UpdateState::Installing => ("正在安装", "正在安装更新"),
            UpdateState::Failed => ("重试更新", "更新未完成"),
        }
    }
    unsafe fn refresh_update_card(h: Handle) {
        let Some(app) = app() else { return };
        let state = (app.update.status)();
        let changed = app.update_state.replace(Some(state)) != Some(state);
        if !changed && !HIGH_CONTRAST {
            return;
        }
        let (compact_title, detail) = update_caption(state);
        let native_label = if HIGH_CONTRAST && viewport_height(h) < SIDEBAR_CARD_MIN_HEIGHT {
            compact_title.to_owned()
        } else if detail.is_empty() {
            format!("检查更新\r\n当前版本 v{}", application_version())
        } else {
            format!("检查更新\r\n{detail}")
        };
        let control = GetDlgItem(h, C_UPDATE as i32);
        if text(h, C_UPDATE) != native_label {
            SetWindowTextW(control, wide(&native_label).as_ptr());
        }
        if changed {
            InvalidateRect(control, null(), 0);
        }
    }
    unsafe fn show_about(h: Handle) {
        let message = format!(
            "校园网守护  v{}\r\n武汉理工大学校园网连接助手\r\n\r\n作者  Liunian520\r\n开源许可  MIT\r\n\r\n{}\r\n侧栏可打开 GitHub 主页或检查更新。",
            application_version(),
            AUTHOR_URL
        );
        MessageBoxW(
            h,
            wide(&message).as_ptr(),
            wide("关于校园网守护").as_ptr(),
            0x40,
        );
    }
    unsafe fn refresh_analytics_consent(h: Handle, app: &App) {
        SendMessageW(
            GetDlgItem(h, C_ANALYTICS as i32),
            BM_SETCHECK,
            usize::from((app.analytics.enabled)()),
            0,
        );
    }
    unsafe fn change_analytics_consent(h: Handle) {
        let Some(app) = app() else { return };
        let checked = SendMessageW(GetDlgItem(h, C_ANALYTICS as i32), BM_GETCHECK, 0, 0) == 1;
        let result = (app.analytics.set_enabled)(checked);
        refresh_analytics_consent(h, app);
        if let Err(error) = result {
            alert(h, &format!("无法保存匿名使用统计设置。\r\n{error}"));
        }
    }
    unsafe fn make_font(height: i32, weight: i32) -> Handle {
        CreateFontW(
            -scale_for_dpi(height, UI_DPI),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            wide("Microsoft YaHei UI").as_ptr(),
        )
    }
    unsafe fn control(
        h: Handle,
        class: &str,
        label: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        height: i32,
        id: usize,
    ) {
        let child = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(label).as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | style
                | if class == "STATIC" { 0x80 } else { 0 }
                | if HIGH_CONTRAST && class == "EDIT" {
                    0x800000
                } else {
                    0
                },
            scale_for_dpi(PANEL_WIDTH.x(x), UI_DPI),
            scale_for_dpi(y, UI_DPI),
            scale_for_dpi((PANEL_WIDTH.x(x + w) - PANEL_WIDTH.x(x)).max(1), UI_DPI),
            scale_for_dpi(height, UI_DPI),
            h,
            id as isize,
            GetModuleHandleW(null()),
            null(),
        );
        if child != 0 {
            let font = if id == C_TITLE || id == C_SETTINGS_TITLE {
                TITLE_FONT
            } else if id == C_STATUS_HEAD {
                HERO_FONT
            } else if matches!(
                id,
                C_SECTION
                    | C_FORM_NETWORK
                    | C_FORM_ACCOUNT
                    | C_FORM_AUTOMATION
                    | C_DIAGNOSTICS_TITLE
            ) {
                SECTION_FONT
            } else if matches!(
                id,
                C_SUBTITLE
                    | C_HINT_NETWORK
                    | C_HINT_PASSWORD
                    | C_FOOTER
                    | C_START_HINT
                    | C_FORM_SUBTITLE
                    | C_NETWORK_DESC
                    | C_OVERVIEW_HINT
                    | C_NETWORK_LABEL
                    | C_STATUS_LABEL
                    | C_HTTP_HINT
                    | C_FORM_FEEDBACK
                    | C_ADAPTER_DETAIL
                    | C_ANALYTICS_DISCLOSURE
                    | C_DIAGNOSTICS_FEEDBACK
                    | C_ABOUT
            ) {
                SMALL_FONT
            } else {
                UI_FONT
            };
            SendMessageW(
                child,
                0x30,
                if font != 0 { font } else { GetStockObject(17) } as usize,
                1,
            );
            LAYOUT.with(|layout| {
                layout.borrow_mut().push(ControlLayout {
                    hwnd: child,
                    x,
                    y,
                    w,
                    h: height,
                    page: BUILD_PAGE,
                })
            });
            SetWindowTheme(child, wide("Explorer").as_ptr(), null());
            if class == "BUTTON" && style & 0xf == 0xb {
                SetWindowSubclass(child, Some(button_proc), 1, 0);
            } else if class == "EDIT" || class == "COMBOBOX" {
                SetWindowSubclass(child, Some(input_proc), 1, 0);
            }
            if class == "EDIT" {
                SendMessageW(child, 0xd3, 3, 0);
            }
        }
    }
    unsafe extern "system" fn button_proc(
        h: Handle,
        m: u32,
        w: usize,
        l: isize,
        _: usize,
        _: usize,
    ) -> isize {
        if m == 0x200 {
            if HOVERED_BUTTON != h {
                if HOVERED_BUTTON != 0 {
                    InvalidateRect(HOVERED_BUTTON, null(), 1);
                }
                HOVERED_BUTTON = h;
                InvalidateRect(h, null(), 1);
            }
            let mut track = TrackMouse {
                size: size_of::<TrackMouse>() as u32,
                flags: 2,
                hwnd: h,
                hover_time: 0,
            };
            TrackMouseEvent(&mut track);
        } else if m == 0x2a3 {
            if HOVERED_BUTTON == h {
                HOVERED_BUTTON = 0;
            }
            InvalidateRect(h, null(), 1);
        }
        DefSubclassProc(h, m, w, l)
    }
    unsafe extern "system" fn input_proc(
        h: Handle,
        m: u32,
        w: usize,
        l: isize,
        _: usize,
        _: usize,
    ) -> isize {
        let id = GetDlgCtrlID(h) as usize;
        if m == 0x87 && matches!(id, C_STATUS | C_DIAGNOSTICS_REPORT) {
            // A multiline EDIT normally claims every key. Keep its arrow and
            // selection handling, but let dialog navigation handle Tab so the
            // read-only text remains accessible without trapping focus.
            let flags = DefSubclassProc(h, m, w, l) & !(0x2 | 0x4);
            // Remove DLGC_HASSETSEL so dialog Tab leaves the history readable.
            return if id == C_DIAGNOSTICS_REPORT {
                flags & !0x8
            } else {
                flags
            };
        }
        // The combo remains a native keyboard- and accessibility-aware dropdown.
        // Only its closed surface is painted to match the rounded input shell.
        if !HIGH_CONTRAST && matches!(id, C_ADAPTER | C_SECONDARY_ADAPTER) {
            if m == 0xf {
                let mut paint: Paint = zeroed();
                let dc = BeginPaint(h, &mut paint);
                paint_combo(h, dc);
                EndPaint(h, &paint);
                return 0;
            } else if m == 0x317 || m == 0x318 {
                paint_combo(h, w as Handle);
                return 0;
            } else if m == 0x14 || m == 0x85 {
                return 1;
            }
        }
        let result = DefSubclassProc(h, m, w, l);
        if m == 7 || m == 8 {
            InvalidateRect(GetParent(h), null(), 1);
        }
        result
    }
    unsafe fn paint_combo(h: Handle, dc: Handle) {
        let mut bounds: Rect = zeroed();
        GetClientRect(h, &mut bounds);
        FillRect(dc, &bounds, CARD_BRUSH);
        let length = GetWindowTextLengthW(h);
        let mut buffer = vec![0u16; (length + 1).max(1) as usize];
        let count = GetWindowTextW(h, buffer.as_mut_ptr(), buffer.len() as i32);
        let selected = String::from_utf16_lossy(&buffer[..count.max(0) as usize]);
        let placeholder = selected.is_empty();
        let arrow_x = bounds.right - scale_for_dpi(15, UI_DPI);
        let middle_y = (bounds.top + bounds.bottom) / 2;
        bounds.left += scale_for_dpi(2, UI_DPI);
        bounds.right -= scale_for_dpi(28, UI_DPI);
        paint_label(
            dc,
            if placeholder {
                if GetDlgCtrlID(h) as usize == C_SECONDARY_ADAPTER {
                    "不守护第二个网络"
                } else {
                    "选择校园网络"
                }
            } else {
                adapter_caption(&selected)
            },
            bounds,
            UI_FONT,
            if placeholder {
                theme::TEXT_TERTIARY
            } else {
                theme::TEXT_PRIMARY
            },
            0x4 | 0x20 | 0x8000,
        );
        let pen = CreatePen(0, scale_for_dpi(1, UI_DPI), theme::TEXT_SECONDARY);
        let old = SelectObject(dc, pen);
        MoveToEx(
            dc,
            arrow_x - scale_for_dpi(4, UI_DPI),
            middle_y - scale_for_dpi(2, UI_DPI),
            null_mut(),
        );
        LineTo(dc, arrow_x, middle_y + scale_for_dpi(2, UI_DPI));
        LineTo(
            dc,
            arrow_x + scale_for_dpi(4, UI_DPI),
            middle_y - scale_for_dpi(2, UI_DPI),
        );
        SelectObject(dc, old);
        DeleteObject(pen);
    }
    fn adapter_caption(name: &str) -> &str {
        name.rsplit_once(" / 网关 ")
            .map_or(name, |(caption, _)| caption)
    }
    unsafe fn single_line_width(dc: Handle, value: &str) -> i32 {
        let mut bounds: Rect = zeroed();
        DrawTextW(
            dc,
            wide(value).as_ptr(),
            -1,
            &mut bounds,
            0x20 | 0x400 | 0x800,
        );
        bounds.right
    }
    unsafe fn network_display_text(h: Handle, value: &str, font: Handle, width: i32) -> String {
        let dc = GetDC(h);
        if dc == 0 {
            return value.to_owned();
        }
        let old = SelectObject(dc, font);
        let available = scale_for_dpi(PANEL_WIDTH.content_span(width).max(1), UI_DPI);
        let mut output = String::new();
        // DrawText word wrapping can leave an unbroken ASCII device name on
        // one oversized line. Insert display-only breaks in those runs; keep
        // the adapter's actual name and saved identity unchanged.
        for part in value.split_inclusive(|c: char| !c.is_ascii() || c.is_ascii_whitespace()) {
            if single_line_width(dc, part) <= available {
                output.push_str(part);
            } else {
                let mut line = String::new();
                for character in part.chars() {
                    let next = format!("{line}{character}");
                    if !line.is_empty() && single_line_width(dc, &next) > available {
                        output.push_str(&line);
                        output.push('\n');
                        line.clear();
                    }
                    line.push(character);
                }
                output.push_str(&line);
            }
        }
        SelectObject(dc, old);
        ReleaseDC(h, dc);
        output
    }
    unsafe fn measured_height(h: Handle, value: &str, font: Handle, width: i32) -> i32 {
        if value.is_empty() {
            return 0;
        }
        let dc = GetDC(h);
        if dc == 0 {
            return 40;
        }
        let old = SelectObject(dc, font);
        let mut bounds = Rect {
            left: 0,
            top: 0,
            right: scale_for_dpi(PANEL_WIDTH.content_span(width).max(1), UI_DPI),
            bottom: 0,
        };
        DrawTextW(
            dc,
            wide(value).as_ptr(),
            -1,
            &mut bounds,
            0x10 | 0x400 | 0x800,
        );
        SelectObject(dc, old);
        ReleaseDC(h, dc);
        // Round up to the 4-DIP grid, including the font's actual line metrics.
        let dip = ((bounds.bottom as i64 * 96 + UI_DPI as i64 - 1) / UI_DPI as i64) as i32;
        ((dip + 3) / 4) * 4
    }
    unsafe fn set_control_bounds(id: usize, x: i32, y: i32, w: i32, height: i32) {
        LAYOUT.with(|layout| {
            if let Some(item) = layout
                .borrow_mut()
                .iter_mut()
                .find(|c| GetDlgCtrlID(c.hwnd) as usize == id)
            {
                item.x = x;
                item.y = y;
                item.w = w;
                item.h = height;
            }
        });
    }
    unsafe fn update_overview_layout(h: Handle) {
        let viewport = viewport_height(h);
        let status_limit = if viewport < COMPACT_OVERVIEW_HEIGHT {
            (viewport - 228).clamp(44, 100)
        } else {
            100
        };
        let status_height =
            measured_height(h, &text(h, C_STATUS), UI_FONT, 472).clamp(44, status_limit);
        HERO_EXTRA = status_height - 44;
        NETWORK_TEXT_HEIGHT = measured_height(h, &text(h, C_NETWORK_VALUE), UI_FONT, 496).max(28);
        NETWORK_DETAIL_HEIGHT =
            measured_height(h, &text(h, C_NETWORK_DESC), SMALL_FONT, 496).max(20);
        let diagnostics_y = DIAGNOSTICS_TOP + HERO_EXTRA;
        let summary_height = DIAGNOSTICS_SUMMARY_HEIGHT;
        let summary_y = diagnostics_y + 48;
        let actions_y = summary_y + summary_height + 12;
        let feedback = text(h, C_DIAGNOSTICS_FEEDBACK);
        let feedback_height = measured_height(h, &feedback, SMALL_FONT, 496);
        let feedback_y = actions_y + 46;
        let report_y = feedback_y
            + if feedback_height > 0 {
                feedback_height + 8
            } else {
                0
            };
        let history_open = app().is_some_and(|app| app.history_open.get());
        let diagnostics_end = if history_open {
            report_y + RECOVERY_REPORT_HEIGHT + 16
        } else if feedback_height > 0 {
            feedback_y + feedback_height + 16
        } else {
            actions_y + 56
        };
        DIAGNOSTICS_HEIGHT = diagnostics_end - diagnostics_y;
        let network_y = diagnostics_end + 16;
        let network_height = 76 + NETWORK_TEXT_HEIGHT + NETWORK_DETAIL_HEIGHT;
        set_control_bounds(C_STATUS, 216, 180, 496, status_height);
        set_control_bounds(M_CHECK, 192, 256 + HERO_EXTRA, 144, 40);
        set_control_bounds(M_OPEN, 352, 256 + HERO_EXTRA, 176, 40);
        set_control_bounds(M_CHECK_EXTERNAL, 544, 256 + HERO_EXTRA, 152, 40);
        set_control_bounds(C_DIAGNOSTICS_TITLE, 216, diagnostics_y + 18, 496, 24);
        set_control_bounds(C_DIAGNOSTICS_SUMMARY, 216, summary_y, 496, summary_height);
        set_control_bounds(C_DIAGNOSTICS_HISTORY, 216, actions_y, 148, 36);
        set_control_bounds(C_DIAGNOSTICS_COPY, 376, actions_y, 116, 36);
        set_control_bounds(
            C_DIAGNOSTICS_FEEDBACK,
            216,
            feedback_y,
            496,
            feedback_height,
        );
        set_control_bounds(
            C_DIAGNOSTICS_REPORT,
            216,
            report_y,
            496,
            RECOVERY_REPORT_HEIGHT,
        );
        set_control_bounds(C_NETWORK_LABEL, 216, network_y + 20, 496, 20);
        set_control_bounds(
            C_NETWORK_VALUE,
            216,
            network_y + 52,
            496,
            NETWORK_TEXT_HEIGHT,
        );
        set_control_bounds(
            C_NETWORK_DESC,
            216,
            network_y + 56 + NETWORK_TEXT_HEIGHT,
            496,
            NETWORK_DETAIL_HEIGHT,
        );
        let notice = text(h, C_OVERVIEW_HINT);
        let notice_height = measured_height(h, &notice, SMALL_FONT, 544);
        set_control_bounds(
            C_OVERVIEW_HINT,
            192,
            network_y + network_height + 16,
            544,
            notice_height,
        );
        let footer_y = network_y
            + network_height
            + 24
            + if notice_height > 0 {
                notice_height + 8
            } else {
                0
            };
        let footer_height = measured_height(h, &text(h, C_FOOTER), SMALL_FONT, 544).max(44);
        set_control_bounds(C_FOOTER, 192, footer_y, 544, footer_height);
        OVERVIEW_HEIGHT = CONTENT_HEIGHT.max(footer_y + footer_height + 24);
        layout_controls(h);
        // Keep complete native EDIT lines, with scrolling for unusually long errors.
        let status_control = GetDlgItem(h, C_STATUS as i32);
        let mut client: Rect = zeroed();
        GetClientRect(status_control, &mut client);
        SendMessageW(status_control, 0xb3, 0, (&client as *const Rect) as isize);
        let line_count = SendMessageW(status_control, 0xba, 0, 0);
        let second_line = SendMessageW(status_control, 0xbb, 1, 0);
        // EDIT messages return a signed 32-bit -1, which can be zero-extended
        // to 0xFFFFFFFF by Windows even in a 64-bit process.
        if line_count > 1 && second_line as i32 >= 0 {
            let first = SendMessageW(status_control, 0xd6, 0, 0);
            let second = SendMessageW(status_control, 0xd6, second_line as usize, 0);
            if first as i32 != -1 && second as i32 != -1 {
                let line =
                    ((second >> 16) as u16 as i16 as i32) - ((first >> 16) as u16 as i16 as i32);
                if line > 0 {
                    client.bottom = client.bottom / line * line;
                    SendMessageW(status_control, 0xb3, 0, (&client as *const Rect) as isize);
                    ShowScrollBar(
                        status_control,
                        1,
                        i32::from(
                            SendMessageW(status_control, 0xba, 0, 0) * line as isize
                                > client.bottom as isize,
                        ),
                    );
                }
            }
        } else {
            ShowScrollBar(status_control, 1, 0);
        }
        if history_open {
            fit_history_report(GetDlgItem(h, C_DIAGNOSTICS_REPORT as i32));
        }
        configure_scroll(h);
    }

    unsafe fn fit_history_report(control: Handle) {
        let mut inner: Rect = zeroed();
        GetClientRect(control, &mut inner);
        let padding = scale_for_dpi(8, UI_DPI);
        inner.left += padding;
        inner.right = (inner.right - padding).max(inner.left + 1);
        inner.top += padding;
        inner.bottom -= padding;
        let first = SendMessageW(control, 0xce, 0, 0) as usize;
        let line_top = |line| {
            let index = SendMessageW(control, 0xbb, line, 0) as i32;
            (index >= 0)
                .then(|| (SendMessageW(control, 0xd6, index as usize, 0) >> 16) as i16 as i32)
        };
        if let (Some(first), Some(second)) = (line_top(first), line_top(first + 1)) {
            let line_height = second - first;
            if line_height > 0 {
                inner.bottom = inner.top + (inner.bottom - inner.top) / line_height * line_height;
            }
        }
        SendMessageW(control, 0xb3, 0, (&inner as *const Rect) as isize);
    }

    unsafe fn measure_form_text(h: Handle) {
        let password = measured_height(h, &text(h, C_HINT_PASSWORD), SMALL_FONT, 504).max(20);
        let consent = measured_height(h, &text(h, C_CONSENT), UI_FONT, 472).max(22);
        let http = measured_height(h, &text(h, C_HTTP_HINT), SMALL_FONT, 482).max(20);
        let startup = measured_height(h, &text(h, C_START), UI_FONT, 472).max(22);
        let startup_hint = measured_height(h, &text(h, C_START_HINT), SMALL_FONT, 482).max(18);
        let analytics = measured_height(h, ANALYTICS_DISCLOSURE, SMALL_FONT, 482);
        FORM_WRAP = FormWrap {
            account: password - 20,
            consent: consent - 22,
            http_hint: http - 20,
            startup: startup - 22,
            automation: (consent - 22) + (http - 20) + (startup - 22) + (startup_hint - 18),
            analytics,
        };
        set_control_bounds(C_HINT_PASSWORD, 212, 320, 504, password);
        set_control_bounds(C_CONSENT, 212, 414, 504, consent);
        set_control_bounds(C_HTTP_HINT, 234, 440, 482, http);
        set_control_bounds(C_START, 212, 472, 504, startup);
        set_control_bounds(C_START_HINT, 234, 496, 482, startup_hint);
        set_control_bounds(
            C_ANALYTICS_DISCLOSURE,
            234,
            ANALYTICS_DISCLOSURE_TOP,
            482,
            analytics,
        );
    }
    unsafe fn form_feedback(h: Handle, message: &str, field: usize) {
        FORM_ERROR_FIELD = if message.is_empty() { 0 } else { field };
        SetWindowTextW(
            GetDlgItem(h, C_FORM_FEEDBACK as i32),
            wide(message).as_ptr(),
        );
        let height = measured_height(h, message, SMALL_FONT, 544);
        FORM_SHIFT = if height == 0 { 0 } else { height + 12 };
        set_control_bounds(C_FORM_FEEDBACK, 192, feedback_section(field), 544, height);
        layout_controls(h);
        configure_scroll(h);
        if field != 0 && !message.is_empty() {
            SetFocus(GetDlgItem(h, field as i32));
        }
        ensure_focus_visible(h);
    }

    fn feedback_section(field: usize) -> i32 {
        match field {
            C_USER | C_PASS => 200,
            C_CONSENT | C_START => 368,
            C_SAVE | C_DELETE => 520,
            _ => 88,
        }
    }

    unsafe fn form_feedback_shift(y: i32) -> i32 {
        if y >= feedback_section(FORM_ERROR_FIELD) {
            FORM_SHIFT
        } else {
            0
        }
    }
    unsafe fn preceding_form_expansion(y: i32) -> i32 {
        (if y >= 200 { FORM_NETWORK_EXTRA } else { 0 })
            + if y >= 368 { FORM_WRAP.account } else { 0 }
            + if y >= 520 { FORM_WRAP.automation } else { 0 }
    }
    unsafe fn viewport_height(h: Handle) -> i32 {
        let mut client: Rect = zeroed();
        GetClientRect(h, &mut client);
        ((client.bottom as i64 * 96) / UI_DPI as i64) as i32
    }
    unsafe fn content_height(h: Handle) -> i32 {
        if ACTIVE_PAGE == 2 {
            ANALYTICS_TOP + analytics_card_height() + 24 + settings_analytics_shift()
        } else if viewport_height(h) < COMPACT_OVERVIEW_HEIGHT {
            OVERVIEW_HEIGHT - COMPACT_OVERVIEW_SHIFT
        } else {
            OVERVIEW_HEIGHT
        }
    }
    unsafe fn analytics_card_height() -> i32 {
        ANALYTICS_DISCLOSURE_TOP - ANALYTICS_TOP + FORM_WRAP.analytics + 16
    }
    unsafe fn settings_analytics_shift() -> i32 {
        form_feedback_shift(ANALYTICS_TOP) + preceding_form_expansion(ANALYTICS_TOP)
    }
    const SIDEBAR_CARD_MIN_HEIGHT: i32 = 368;
    unsafe fn layout_controls(h: Handle) {
        let viewport_height = viewport_height(h);
        let compact_overview = viewport_height < COMPACT_OVERVIEW_HEIGHT;
        let history_open = app().is_some_and(|app| app.history_open.get());
        let compact_sidebar = viewport_height < 328;
        let footer_visible = viewport_height >= 208;
        let split_footer = viewport_height < 272;
        let tall_footer = viewport_height >= SIDEBAR_CARD_MIN_HEIGHT;
        let footer_top = if split_footer {
            viewport_height - 52
        } else if tall_footer {
            viewport_height - 152
        } else {
            viewport_height - 108
        };
        if !footer_visible
            && matches!(
                GetDlgCtrlID(GetFocus()) as usize,
                C_AUTHOR | C_UPDATE | C_ABOUT
            )
        {
            SetFocus(GetDlgItem(
                h,
                if ACTIVE_PAGE == 2 {
                    C_NAV_SETTINGS
                } else {
                    C_NAV_OVERVIEW
                } as i32,
            ));
        }
        LAYOUT.with(|layout| {
            for item in layout.borrow().iter() {
                let id = GetDlgCtrlID(item.hwnd) as usize;
                if id == C_AUTHOR && HIGH_CONTRAST {
                    let label = if split_footer {
                        "作者"
                    } else if tall_footer {
                        "Liunian520\r\n作者 · GitHub"
                    } else {
                        "Liunian520"
                    };
                    if text(h, C_AUTHOR) != label {
                        SetWindowTextW(item.hwnd, wide(label).as_ptr());
                    }
                }
                let visible = (item.page == 0 || item.page == ACTIVE_PAGE)
                    && (!matches!(id, C_AUTHOR | C_UPDATE | C_ABOUT) || footer_visible)
                    && !(compact_overview && matches!(id, C_SUBTITLE | C_STATUS_LABEL))
                    && (id != C_DIAGNOSTICS_REPORT || history_open);
                ShowWindow(item.hwnd, if visible { 5 } else { 0 });
                let offset = if item.page == 0 { 0 } else { SCROLL_OFFSET };
                let (x, y, width, height) = match id {
                    C_NAV_OVERVIEW if compact_sidebar => (item.x, 88, item.w, 32),
                    C_NAV_SETTINGS if compact_sidebar => (item.x, 124, item.w, 32),
                    C_AUTHOR if split_footer => (12, footer_top, 68, 24),
                    C_UPDATE if split_footer => (84, footer_top, 72, 24),
                    C_ABOUT if split_footer => (12, footer_top + 26, 144, 24),
                    C_AUTHOR if tall_footer => (12, footer_top, 144, 52),
                    C_UPDATE if tall_footer => (12, footer_top + 60, 144, 52),
                    C_ABOUT if tall_footer => (12, footer_top + 120, 144, 24),
                    C_AUTHOR => (12, footer_top, 144, 32),
                    C_UPDATE => (12, footer_top + 36, 144, 32),
                    C_ABOUT => (12, footer_top + 72, 144, 24),
                    _ => (item.x, item.y, item.w, item.h),
                };
                let mut shift = if item.page == 2
                    && !matches!(id, C_SETTINGS_TITLE | C_FORM_SUBTITLE | C_FORM_FEEDBACK)
                {
                    form_feedback_shift(if id == C_ADAPTER_DETAIL { 88 } else { item.y })
                        + if item.y >= 200 && id != C_ADAPTER_DETAIL {
                            FORM_NETWORK_EXTRA
                        } else {
                            0
                        }
                } else {
                    0
                };
                if item.page == 2 && item.y >= 368 && id != C_FORM_FEEDBACK {
                    shift += FORM_WRAP.account;
                    shift += match id {
                        C_HTTP_HINT => FORM_WRAP.consent,
                        C_START => FORM_WRAP.consent + FORM_WRAP.http_hint,
                        C_START_HINT => FORM_WRAP.consent + FORM_WRAP.http_hint + FORM_WRAP.startup,
                        C_SAVE | C_DELETE | C_ANALYTICS | C_ANALYTICS_DISCLOSURE => {
                            FORM_WRAP.automation
                        }
                        _ => 0,
                    };
                }
                if id == C_FORM_FEEDBACK {
                    shift += preceding_form_expansion(item.y);
                }
                if compact_overview && item.page == 1 && id != C_TITLE {
                    shift -= COMPACT_OVERVIEW_SHIFT;
                }
                let left = scale_for_dpi(PANEL_WIDTH.x(x), UI_DPI);
                let right = scale_for_dpi(PANEL_WIDTH.x(x + width), UI_DPI);
                MoveWindow(
                    item.hwnd,
                    left,
                    scale_for_dpi(y + shift, UI_DPI) - offset,
                    (right - left).max(1),
                    scale_for_dpi(height, UI_DPI),
                    1,
                );
            }
        });
        if HIGH_CONTRAST {
            refresh_update_card(h);
        }
        InvalidateRect(h, null(), 1);
    }
    unsafe fn show_page(h: Handle, page: u8) {
        let focus = GetFocus();
        let transfer = LAYOUT.with(|layout| {
            layout
                .borrow()
                .iter()
                .any(|c| c.hwnd == focus && c.page != 0 && c.page != page)
        });
        HOVERED_BUTTON = 0;
        SCROLL_REVEALED_UNTIL = None;
        let bar = GetDlgItem(h, C_PAGE_SCROLL as i32);
        if bar != 0 {
            KillTimer(bar, SCROLL_REVEAL_TIMER);
        }
        ACTIVE_PAGE = page;
        // A native button exposes its window text as its accessible name. Keep
        // the current page explicit for assistive technology as well as through
        // the painted selection indicator.
        SetWindowTextW(
            GetDlgItem(h, C_NAV_OVERVIEW as i32),
            wide(if page == 1 {
                "连接概览，当前页"
            } else {
                "连接概览"
            })
            .as_ptr(),
        );
        SetWindowTextW(
            GetDlgItem(h, C_NAV_SETTINGS as i32),
            wide(if page == 2 {
                "认证设置，当前页"
            } else {
                "认证设置"
            })
            .as_ptr(),
        );
        if page == 2 {
            refresh_adapters(h);
            if let Some(app) = app() {
                refresh_analytics_consent(h, app);
            }
        } else {
            update_overview(h);
        }
        scroll_to(h, 0);
        layout_controls(h);
        configure_scroll(h);
        if transfer {
            SetFocus(GetDlgItem(
                h,
                if page == 2 { C_ADAPTER } else { M_CHECK } as i32,
            ));
        }
    }
    unsafe fn open_panel(h: Handle, page: u8) {
        show_page(h, page);
        // SW_RESTORE also clears the minimized state. The launcher's initial
        // STARTUPINFO may override the first top-level ShowWindow with SW_HIDE;
        // repeat it so one tray activation always makes the panel visible.
        ShowWindow(h, 9);
        ShowWindow(h, 9);
        SetForegroundWindow(h);
    }
    unsafe fn refresh_adapters(h: Handle) {
        if let Some(app) = app() {
            let (primary, secondary) = {
                let adapters = app.adapters.borrow();
                (
                    chosen_adapter(h, C_ADAPTER, &adapters).map(|adapter| adapter.id.clone()),
                    chosen_adapter(h, C_SECONDARY_ADAPTER, &adapters)
                        .map(|adapter| adapter.id.clone()),
                )
            };
            populate_adapter_choices(
                h,
                app,
                primary.as_deref().unwrap_or_default(),
                secondary.as_deref().unwrap_or_default(),
            );
        }
    }
    unsafe fn chosen_adapter(h: Handle, id: usize, adapters: &[Adapter]) -> Option<&Adapter> {
        let index = SendMessageW(GetDlgItem(h, id as i32), 0x147, 0, 0) as i32;
        let offset = usize::from(id == C_SECONDARY_ADAPTER);
        adapters.get(usize::try_from(index).ok()?.checked_sub(offset)?)
    }
    unsafe fn populate_adapter_choices(h: Handle, app: &App, primary: &str, secondary: &str) {
        let adapters = adapter_options(
            (app.adapter_source)(),
            &app.settings.borrow(),
            [primary, secondary],
        );
        for (id, selected) in [(C_ADAPTER, primary), (C_SECONDARY_ADAPTER, secondary)] {
            let combo = GetDlgItem(h, id as i32);
            let optional = id == C_SECONDARY_ADAPTER;
            SendMessageW(combo, 0x14b, 0, 0);
            if optional {
                SendMessageW(combo, 0x143, 0, wide("不守护第二个网络").as_ptr() as isize);
            }
            for adapter in &adapters {
                SendMessageW(combo, 0x143, 0, wide(&adapter.name).as_ptr() as isize);
            }
            let index = adapters
                .iter()
                .position(|adapter| adapter.id == selected)
                .map(|position| position + usize::from(optional))
                .unwrap_or(if optional { 0 } else { usize::MAX });
            SendMessageW(combo, 0x14e, index, 0);
        }
        *app.adapters.borrow_mut() = adapters;
        update_adapter_detail(h);
    }
    unsafe fn reset_form(h: Handle) {
        if let Some(app) = app() {
            let settings = app.settings.borrow().clone();
            populate_adapter_choices(h, app, &settings.adapter_id, &settings.secondary_adapter_id);
            SetWindowTextW(
                GetDlgItem(h, C_USER as i32),
                wide(&settings.username).as_ptr(),
            );
            SetWindowTextW(GetDlgItem(h, C_PASS as i32), wide("").as_ptr());
            SendMessageW(
                GetDlgItem(h, C_CONSENT as i32),
                0xf1,
                settings.http_acknowledged as usize,
                0,
            );
            SendMessageW(
                GetDlgItem(h, C_START as i32),
                0xf1,
                settings.autostart as usize,
                0,
            );
        }
    }
    unsafe fn update_adapter_detail(h: Handle) {
        let Some(app) = app() else { return };
        let adapters = app.adapters.borrow();
        let dc = GetDC(h);
        let old_font = if dc == 0 {
            0
        } else {
            SelectObject(dc, UI_FONT)
        };
        let detail = [(C_ADAPTER, "主网络"), (C_SECONDARY_ADAPTER, "第二个网络")]
            .into_iter()
            .filter_map(|(id, label)| {
                let adapter = chosen_adapter(h, id, &adapters)?;
                let caption = adapter_caption(&adapter.name);
                (dc == 0
                    || single_line_width(dc, caption)
                        > scale_for_dpi(
                            (PANEL_WIDTH.content_span(ADAPTER_COMBO_WIDTH) - 30).max(1),
                            UI_DPI,
                        ))
                .then(|| format!("{label}　{}", adapter.name))
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        drop(adapters);
        if dc != 0 {
            SelectObject(dc, old_font);
            ReleaseDC(h, dc);
        }
        let detail = network_display_text(h, &detail, SMALL_FONT, 504);
        let height = measured_height(h, &detail, SMALL_FONT, 504);
        FORM_NETWORK_EXTRA = NETWORK_FIELD_ROW_HEIGHT + if height == 0 { 0 } else { height + 12 };
        SetWindowTextW(
            GetDlgItem(h, C_ADAPTER_DETAIL as i32),
            wide(&detail).as_ptr(),
        );
        set_control_bounds(C_ADAPTER_DETAIL, 212, 208, 504, height);
        layout_controls(h);
        configure_scroll(h);
    }
    unsafe fn update_overview(h: Handle) {
        if ACTIVE_PAGE != 1 {
            return;
        }
        if let Some(app) = app() {
            let status = app
                .status
                .lock()
                .map(|value| value.clone())
                .unwrap_or_else(|_| "状态不可用".into());
            let headline = connection_heading(&status);
            SetWindowTextW(GetDlgItem(h, C_STATUS_HEAD as i32), wide(headline).as_ptr());
            // The settings list is a draft snapshot, including unavailable
            // options. Read the current adapters without changing that list.
            let adapters = (app.adapter_source)();
            let settings = app.settings.borrow();
            let dual = !settings.secondary_adapter_id.is_empty();
            let (network, detail) = overview_network_text(&settings, &adapters);
            drop(settings);
            SetWindowTextW(
                GetDlgItem(h, C_NETWORK_LABEL as i32),
                wide(if dual {
                    "已选校园网络"
                } else {
                    "当前校园网络"
                })
                .as_ptr(),
            );
            SetWindowTextW(
                GetDlgItem(h, C_NETWORK_VALUE as i32),
                wide(&network_display_text(h, &network, UI_FONT, 496)).as_ptr(),
            );
            SetWindowTextW(GetDlgItem(h, C_NETWORK_DESC as i32), wide(&detail).as_ptr());
            refresh_diagnostics(h, app);
            update_overview_layout(h);
            InvalidateRect(h, null(), 1);
        }
    }
    unsafe fn refresh_diagnostics(h: Handle, app: &App) -> bool {
        let summary = (app.diagnostics.summary)();
        let summary = if summary.is_empty() {
            "等待首次诊断。"
        } else {
            &summary
        };
        let previous_height = DIAGNOSTICS_SUMMARY_HEIGHT;
        if text(h, C_DIAGNOSTICS_SUMMARY) != summary {
            SetWindowTextW(
                GetDlgItem(h, C_DIAGNOSTICS_SUMMARY as i32),
                wide(summary).as_ptr(),
            );
            DIAGNOSTICS_SUMMARY_HEIGHT = measured_height(h, summary, UI_FONT, 496).max(20);
        }
        if app.history_open.get() {
            let report = (app.diagnostics.report)();
            // The inline view starts at the history. Clipboard copying uses the full report.
            let history = report
                .match_indices("最近恢复记录")
                .find(|(offset, _)| *offset == 0 || report.as_bytes()[offset - 1] == b'\n')
                .map_or(report.as_str(), |(offset, _)| &report[offset..]);
            if text(h, C_DIAGNOSTICS_REPORT) != history {
                let control = GetDlgItem(h, C_DIAGNOSTICS_REPORT as i32);
                let mut selection_start = 0u32;
                let mut selection_end = 0u32;
                SendMessageW(
                    control,
                    0xb0,
                    (&mut selection_start as *mut u32) as usize,
                    (&mut selection_end as *mut u32) as isize,
                );
                let first_line = SendMessageW(control, 0xce, 0, 0);
                SetWindowTextW(control, wide(history).as_ptr());
                SendMessageW(
                    control,
                    0xb1,
                    selection_start as usize,
                    selection_end as isize,
                );
                SendMessageW(
                    control,
                    0xb6,
                    0,
                    first_line - SendMessageW(control, 0xce, 0, 0),
                );
            }
        }
        previous_height != DIAGNOSTICS_SUMMARY_HEIGHT
    }
    unsafe fn copy_to_clipboard(h: Handle, report: &str) -> bool {
        const GMEM_MOVEABLE: u32 = 0x0002;
        const CF_UNICODETEXT: u32 = 13;
        let utf16 = wide(report);
        let memory = GlobalAlloc(GMEM_MOVEABLE, utf16.len() * size_of::<u16>());
        if memory == 0 {
            return false;
        }
        let buffer = GlobalLock(memory).cast::<u16>();
        if buffer.is_null() {
            GlobalFree(memory);
            return false;
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), buffer, utf16.len());
        GlobalUnlock(memory);
        let copied = if OpenClipboard(h) != 0 {
            let copied = EmptyClipboard() != 0 && SetClipboardData(CF_UNICODETEXT, memory) != 0;
            CloseClipboard();
            copied
        } else {
            false
        };
        // Windows owns the block once SetClipboardData succeeds.
        if !copied {
            GlobalFree(memory);
        }
        copied
    }
    unsafe fn copy_diagnostics(h: Handle, app: &App) {
        let report = (app.diagnostics.report)();
        let preview = cfg!(debug_assertions) && std::env::var_os("CAMPUS_UI_PREVIEW_DPI").is_some();
        let feedback = if preview {
            "演示诊断已生成。"
        } else if copy_to_clipboard(h, &report) {
            "已复制诊断。"
        } else {
            "无法复制诊断，请重试。"
        };
        SetWindowTextW(
            GetDlgItem(h, C_DIAGNOSTICS_FEEDBACK as i32),
            wide(feedback).as_ptr(),
        );
        update_overview_layout(h);
        ensure_focus_visible(h);
    }
    unsafe fn save_form(h: Handle) {
        if let Some(app) = app() {
            let (id, secondary_id) = {
                let adapters = app.adapters.borrow();
                let selected = |field| {
                    chosen_adapter(h, field, &adapters)
                        .map(|a| a.id.clone())
                        .unwrap_or_default()
                };
                (selected(C_ADAPTER), selected(C_SECONDARY_ADAPTER))
            };
            let username = text(h, C_USER);
            let consent = SendMessageW(GetDlgItem(h, C_CONSENT as i32), 0xf0, 0, 0) == 1;
            let autostart = SendMessageW(GetDlgItem(h, C_START as i32), 0xf0, 0, 0) == 1;
            if !settings_allowed(&id, &secondary_id, &username, consent) {
                let (message, field) = if id.trim().is_empty() {
                    ("请选择当前连接校园网的网卡。", C_ADAPTER)
                } else if id == secondary_id {
                    (
                        "主网络和第二个网络选择了同一张网卡。请选择另一张网卡，或取消第二个网络。",
                        C_SECONDARY_ADAPTER,
                    )
                } else if username.trim().is_empty() {
                    ("请填写校园网账号。", C_USER)
                } else {
                    ("请先阅读并同意 HTTP 明文传输风险，再保存设置。", C_CONSENT)
                };
                form_feedback(h, message, field);
                return;
            }
            let password = Secret(text(h, C_PASS));
            let old_username = app.settings.borrow().username.clone();
            if password.0.is_empty() && old_username != username.trim() {
                form_feedback(h, "更换账号时请重新填写该账号的密码。", C_PASS);
                return;
            }
            let old_password = if old_username.is_empty() {
                None
            } else {
                match read_password(&old_username) {
                    Ok(password) => password.map(Secret),
                    Err(e) => {
                        report_preflight_failure(h, app, &e);
                        return;
                    }
                }
            };
            if password.0.is_empty() && old_password.is_none() {
                form_feedback(h, "请填写密码；当前账号没有保存的凭据。", C_PASS);
                return;
            }
            let previous_autostart = match snapshot_autostart() {
                Ok(value) => value,
                Err(e) => {
                    report_preflight_failure(h, app, &e);
                    return;
                }
            };
            let settings = Settings {
                adapter_id: id,
                secondary_adapter_id: secondary_id,
                username: username.trim().to_string(),
                http_acknowledged: consent,
                autostart,
            };
            if let Err(e) = invalidate_inflight(app) {
                report_settings_failure(h, app, &e);
                return;
            }
            let result = settings_transaction(
                || {
                    if !password.0.is_empty() {
                        write_password(username.trim(), &password.0)?;
                    }
                    set_autostart(autostart)
                },
                || {
                    let (reply_tx, reply_rx) = mpsc::channel();
                    app.tx
                        .send(Event::SettingsSaved(settings.clone(), reply_tx))
                        .map_err(|_| "控制器不可用，设置未保存".to_string())?;
                    reply_rx
                        .recv()
                        .map_err(|_| "控制器未确认设置，设置未保存".to_string())?
                },
                || {
                    let credential_result = if password.0.is_empty() {
                        Ok(())
                    } else if let Some(old) = &old_password {
                        write_password(&old_username, &old.0)
                    } else {
                        delete_password()
                    };
                    let startup_result = restore_autostart(&previous_autostart);
                    match (credential_result, startup_result) {
                        (Ok(()), Ok(())) => Ok(()),
                        (Err(a), Ok(())) | (Ok(()), Err(a)) => Err(a),
                        (Err(a), Err(b)) => Err(format!("{a}；{b}")),
                    }
                },
            );
            match result {
                Ok(()) => {
                    SetWindowTextW(GetDlgItem(h, C_PASS as i32), wide("").as_ptr());
                    form_feedback(h, "", 0);
                    *app.settings.borrow_mut() = settings;
                    SetWindowTextW(
                        GetDlgItem(h, C_OVERVIEW_HINT as i32),
                        wide("设置已保存。连接状态以最新检查结果为准。").as_ptr(),
                    );
                    show_page(h, 1);
                }
                Err(e) => report_settings_failure(h, app, &e),
            }
        }
    }
    unsafe fn register_tray(tray: &mut Notify) -> bool {
        tray.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        // TaskbarCreated is also sent on some DPI changes where the icon remains.
        let available =
            Shell_NotifyIconW(NIM_ADD, tray) != 0 || Shell_NotifyIconW(NIM_MODIFY, tray) != 0;
        if available {
            tray.version_or_timeout = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, tray);
        }
        available
    }
    fn overview_footer(tray_available: bool) -> &'static str {
        if tray_available {
            "正常时每 60 秒检查，网络变化后重新确认。\r\n关闭窗口后继续在系统托盘运行。"
        } else {
            "正常时每 60 秒检查，网络变化后重新确认。\r\n系统托盘不可用，关闭窗口会退出守护。"
        }
    }
    unsafe fn tray_menu_position(h: Handle, use_icon: bool, right_aligned: bool) -> Point {
        if use_icon {
            let mut icon: NotifyIconIdentifier = zeroed();
            icon.size = size_of::<NotifyIconIdentifier>() as u32;
            icon.hwnd = h;
            icon.id = TRAY_ICON_ID;
            let mut rect: Rect = zeroed();
            if Shell_NotifyIconGetRect(&icon, &mut rect) == 0 {
                return Point {
                    x: if right_aligned { rect.right } else { rect.left },
                    y: rect.top,
                };
            }
        }
        let mut point = Point { x: 0, y: 0 };
        GetCursorPos(&mut point);
        point
    }
    unsafe fn menu(h: Handle, use_icon: bool) {
        const SM_MENUDROPALIGNMENT: i32 = 40;
        const TPM_RIGHTALIGN: u32 = 0x08;
        const TPM_RIGHTBUTTON: u32 = 0x02;
        const TPM_RETURNCMD: u32 = 0x100;
        const WM_NULL: u32 = 0;
        if let Some(app) = app() {
            let m = CreatePopupMenu();
            if m == 0 {
                return;
            }
            let status = app
                .status
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "状态不可用".into());
            let panel_label = format!("打开面板 · {}", connection_heading(&status));
            AppendMenuW(m, 0, M_TRAY_PANEL, wide(&panel_label).as_ptr());
            AppendMenuW(m, 0, M_CHECK, wide("立即检查").as_ptr());
            AppendMenuW(m, 0, M_SETTINGS, wide("设置").as_ptr());
            let analytics_label = if (app.analytics.enabled)() {
                "匿名使用统计 · 已开启"
            } else {
                "匿名使用统计 · 已关闭"
            };
            AppendMenuW(m, 0, M_ANALYTICS, wide(analytics_label).as_ptr());
            AppendMenuW(m, 0, M_OPEN, wide("认证页面").as_ptr());
            AppendMenuW(m, 0, M_QUIT, wide("退出").as_ptr());
            let right_aligned = GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0;
            let point = tray_menu_position(h, use_icon, right_aligned);
            let flags =
                TPM_RIGHTBUTTON | TPM_RETURNCMD | if right_aligned { TPM_RIGHTALIGN } else { 0 };
            SetForegroundWindow(h);
            let command = TrackPopupMenu(m, flags, point.x, point.y, 0, h, null());
            // The benign message prevents the next tray popup closing immediately.
            PostMessageW(h, WM_NULL, 0, 0);
            DestroyMenu(m);
            if command != 0 {
                PostMessageW(h, WM_COMMAND, command as usize, 0);
            } else if app.tray_available.get() {
                Shell_NotifyIconW(NIM_SETFOCUS, &mut *app.tray.borrow_mut());
            }
        }
    }
    unsafe fn refresh_status() {
        if let Some(app) = app() {
            let status = app
                .status
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "状态不可用".into());
            let mut previous = app.last_status.borrow_mut();
            if *previous == status {
                return;
            }
            let notify = should_notify(&previous, &status);
            *previous = status.clone();
            drop(previous);
            let mut tray = app.tray.borrow_mut();
            SetWindowTextW(
                GetDlgItem(tray.hwnd, C_STATUS as i32),
                wide(&status).as_ptr(),
            );
            update_overview(tray.hwnd);
            tray.flags = NIF_TIP | NIF_SHOWTIP | if notify { NIF_INFO } else { 0 };
            copy_utf16(
                &format!("校园网守护 · {}", connection_heading(&status)),
                &mut tray.tip,
            );
            if notify {
                copy_utf16("校园网守护", &mut tray.info_title);
                copy_utf16(&status, &mut tray.info);
                tray.info_flags = 2;
            }
            if app.tray_available.get() {
                Shell_NotifyIconW(NIM_MODIFY, &mut *tray);
            }
        }
    }
    unsafe fn always_show_page_scrollbar() -> bool {
        #[cfg(debug_assertions)]
        if let Ok(mode) = std::env::var("CAMPUS_UI_PREVIEW_SCROLL_MODE") {
            return mode == "always";
        }
        // Windows mirrors its always-show accessibility setting here.
        // Missing or unrecognized values keep the app's automatic behavior.
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Control Panel\\Accessibility").as_ptr(),
            wide("DynamicScrollbars").as_ptr(),
            0x10,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        ) == 0
            && size == size_of::<u32>() as u32
            && value == 0
    }
    unsafe fn paint_page_scrollbar(bar: Handle, dc: Handle) {
        let mut bounds: Rect = zeroed();
        GetClientRect(bar, &mut bounds);
        FillRect(dc, &bounds, BACKGROUND_BRUSH);
        let focused = GetFocus() == bar;
        let pressed = GetCapture() == bar;
        let active = SCROLL_HOVERED || focused || pressed;
        if !active
            && !SCROLL_ALWAYS_VISIBLE
            && !SCROLL_REVEALED_UNTIL.is_some_and(|until| Instant::now() < until)
        {
            return;
        }
        let mut geometry: ScrollBarInfo = zeroed();
        geometry.size = size_of::<ScrollBarInfo>() as u32;
        if GetScrollBarInfo(bar, OBJID_CLIENT, &mut geometry) == 0
            || geometry.thumb_bottom <= geometry.thumb_top
        {
            // Offscreen and print-only controls may have a valid native range
            // without exposing any bounding rectangles through accessibility.
            let mut info: ScrollInfo = zeroed();
            info.size = size_of::<ScrollInfo>() as u32;
            info.mask = 0x1 | 0x2 | 0x4;
            if GetScrollInfo(bar, SB_CTL, &mut info) == 0 {
                return;
            }
            let length = bounds.bottom - bounds.top;
            let range = (info.max - info.min + 1).max(1);
            let limit = (range - info.page as i32).max(1);
            geometry.arrow_size = GetSystemMetrics(20).min(length / 4);
            let track = length - 2 * geometry.arrow_size;
            let size = (track * info.page as i32 / range)
                .max(GetSystemMetrics(9))
                .min(track);
            geometry.thumb_top =
                geometry.arrow_size + (track - size) * (info.position - info.min) / limit;
            geometry.thumb_bottom = geometry.thumb_top + size;
        }
        let center = (bounds.left + bounds.right) / 2;
        if active {
            let inset = scale_for_dpi(1, UI_DPI);
            let track = Rect {
                left: bounds.left + inset,
                right: bounds.right - inset,
                top: bounds.top,
                bottom: bounds.bottom,
            };
            rounded(
                dc,
                &track,
                10,
                theme::PAGE_SCROLL_TRACK,
                theme::PAGE_SCROLL_TRACK,
            );

            let size = scale_for_dpi(3, UI_DPI);
            let pen = CreatePen(0, scale_for_dpi(1, UI_DPI), theme::PAGE_SCROLL_THUMB_HOVER);
            let old_pen = SelectObject(dc, pen);
            for (y, direction) in [
                (bounds.top + geometry.arrow_size / 2, -1),
                (bounds.bottom - geometry.arrow_size / 2, 1),
            ] {
                MoveToEx(dc, center - size, y - direction, null_mut());
                LineTo(dc, center, y + direction * size);
                LineTo(dc, center + size, y - direction);
            }
            SelectObject(dc, old_pen);
            DeleteObject(pen);
        }
        if geometry.thumb_bottom > geometry.thumb_top {
            let width = scale_for_dpi(if active { 6 } else { 3 }, UI_DPI)
                .min((bounds.right - bounds.left).max(1));
            let thumb = Rect {
                left: center - width / 2,
                right: center - width / 2 + width,
                top: bounds.top + geometry.thumb_top,
                bottom: bounds.top + geometry.thumb_bottom,
            };
            let color = if pressed {
                theme::PAGE_SCROLL_THUMB_PRESSED
            } else if active {
                theme::PAGE_SCROLL_THUMB_HOVER
            } else {
                theme::PAGE_SCROLL_THUMB
            };
            rounded(dc, &thumb, if active { 6 } else { 3 }, color, color);
        }
        if focused {
            bounds.left += 1;
            bounds.top += 1;
            bounds.right -= 1;
            bounds.bottom -= 1;
            focus_outline(dc, &bounds, 9, theme::FOCUS);
        }
    }
    unsafe extern "system" fn page_scrollbar_proc(
        bar: Handle,
        message: u32,
        w: usize,
        l: isize,
        _: usize,
        _: usize,
    ) -> isize {
        // The native control supplies range, thumb tracking, keyboard behavior,
        // and the accessibility object. Replace only its light-theme painting.
        match message {
            0xf => {
                let mut paint: Paint = zeroed();
                let dc = BeginPaint(bar, &mut paint);
                paint_page_scrollbar(bar, dc);
                EndPaint(bar, &paint);
                return 0;
            }
            0x317 | 0x318 => {
                paint_page_scrollbar(bar, w as Handle);
                return 0;
            }
            WM_ERASEBKGND => return 1,
            WM_TIMER if w == SCROLL_REVEAL_TIMER => {
                if !SCROLL_REVEALED_UNTIL.is_some_and(|until| Instant::now() < until) {
                    SCROLL_REVEALED_UNTIL = None;
                    KillTimer(bar, SCROLL_REVEAL_TIMER);
                    InvalidateRect(bar, null(), 0);
                }
                return 0;
            }
            0x200 if !SCROLL_HOVERED => {
                SCROLL_HOVERED = true;
                let mut tracking = TrackMouse {
                    size: size_of::<TrackMouse>() as u32,
                    flags: 2,
                    hwnd: bar,
                    hover_time: 0,
                };
                TrackMouseEvent(&mut tracking);
                InvalidateRect(bar, null(), 0);
            }
            0x2a3 => {
                SCROLL_HOVERED = false;
                InvalidateRect(bar, null(), 0);
            }
            WM_DESTROY => {
                KillTimer(bar, SCROLL_REVEAL_TIMER);
                SCROLL_REVEALED_UNTIL = None;
                SCROLL_HOVERED = false;
            }
            _ => {}
        }
        let result = DefSubclassProc(bar, message, w, l);
        if matches!(message, 7 | 8 | 0x201 | 0x202 | 0x215) {
            InvalidateRect(bar, null(), 0);
        }
        result
    }
    // The fixed-size controls remain readable at the user's DPI; on short screens
    // the form scrolls instead of scaling text down or clipping the last actions.
    unsafe fn configure_scroll(h: Handle) {
        let bar = GetDlgItem(h, C_PAGE_SCROLL as i32);
        if bar == 0 {
            return;
        }
        let mut client: Rect = zeroed();
        GetClientRect(h, &mut client);
        let content = scale_for_dpi(content_height(h), UI_DPI);
        let viewport = client.bottom.max(1);
        let info = ScrollInfo {
            size: size_of::<ScrollInfo>() as u32,
            mask: 0x1 | 0x2,
            min: 0,
            max: content - 1,
            page: viewport as u32,
            position: 0,
            track_position: 0,
        };
        let width = if HIGH_CONTRAST {
            GetSystemMetrics(2)
        } else {
            scale_for_dpi(16, UI_DPI)
        };
        let gutter = scale_for_dpi(2, UI_DPI);
        let content_edge = scale_for_dpi(PANEL_WIDTH.x(736), UI_DPI);
        let width = width.min((client.right - content_edge - 2 * gutter).max(1));
        let margin = scale_for_dpi(12, UI_DPI).min(viewport / 4);
        MoveWindow(
            bar,
            client.right - width - gutter,
            margin,
            width,
            viewport - 2 * margin,
            0,
        );
        SetScrollInfo(bar, SB_CTL, &info, 0);
        if content > viewport {
            ShowWindow(bar, 5);
        } else {
            if GetFocus() == bar {
                let nav = if ACTIVE_PAGE == 2 {
                    C_NAV_SETTINGS
                } else {
                    C_NAV_OVERVIEW
                };
                SetFocus(GetDlgItem(h, nav as i32));
            }
            SCROLL_HOVERED = false;
            SCROLL_REVEALED_UNTIL = None;
            KillTimer(bar, SCROLL_REVEAL_TIMER);
            ShowWindow(bar, 0);
        }
        // A larger viewport can clamp the scrollbar position; keep the child
        // controls and the painted cards aligned with that new position.
        scroll_to(h, SCROLL_OFFSET);
    }
    unsafe fn scroll_to(h: Handle, position: i32) {
        let bar = GetDlgItem(h, C_PAGE_SCROLL as i32);
        if bar == 0 {
            return;
        }
        let info = ScrollInfo {
            size: size_of::<ScrollInfo>() as u32,
            mask: 0x4,
            min: 0,
            max: 0,
            page: 0,
            position,
            track_position: 0,
        };
        let actual = SetScrollInfo(bar, SB_CTL, &info, i32::from(HIGH_CONTRAST));
        if SCROLL_OFFSET != actual {
            SCROLL_OFFSET = actual;
            layout_controls(h);
        }
        InvalidateRect(bar, null(), 0);
    }
    unsafe fn scroll_from_input(h: Handle, position: i32) {
        let before = SCROLL_OFFSET;
        scroll_to(h, position);
        if !HIGH_CONTRAST && SCROLL_OFFSET != before {
            let bar = GetDlgItem(h, C_PAGE_SCROLL as i32);
            SCROLL_REVEALED_UNTIL =
                Some(Instant::now() + Duration::from_millis(u64::from(SCROLL_REVEAL_MS)));
            if SetTimer(bar, SCROLL_REVEAL_TIMER, SCROLL_REVEAL_MS, null()) == 0 {
                SCROLL_REVEALED_UNTIL = None;
            }
            InvalidateRect(bar, null(), 0);
        }
    }
    unsafe fn ensure_focus_visible(h: Handle) {
        let focus = GetFocus();
        if focus == 0 || GetParent(focus) != h {
            return;
        }
        if GetDlgCtrlID(focus) as usize == C_PAGE_SCROLL {
            return;
        }
        if matches!(
            GetDlgCtrlID(focus) as usize,
            C_NAV_OVERVIEW | C_NAV_SETTINGS | C_AUTHOR | C_UPDATE | C_ABOUT
        ) {
            return;
        }
        let mut control = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut client = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(focus, &mut control) == 0 || GetClientRect(h, &mut client) == 0 {
            return;
        }
        MapWindowPoints(0, h, (&mut control as *mut Rect).cast(), 2);
        let margin = scale_for_dpi(8, UI_DPI);
        let focus_id = GetDlgCtrlID(focus) as usize;
        let companion = if focus_id == C_ANALYTICS {
            C_ANALYTICS_DISCLOSURE
        } else if focus_id == C_DIAGNOSTICS_HISTORY
            && app().is_some_and(|app| app.history_open.get())
        {
            C_DIAGNOSTICS_REPORT
        } else if focus_id == C_DIAGNOSTICS_COPY && !text(h, C_DIAGNOSTICS_FEEDBACK).is_empty() {
            C_DIAGNOSTICS_FEEDBACK
        } else if ACTIVE_PAGE == 2 && FORM_SHIFT > 0 && focus_id == FORM_ERROR_FIELD {
            C_FORM_FEEDBACK
        } else {
            0
        };
        if companion != 0 {
            let mut feedback: Rect = zeroed();
            if GetWindowRect(GetDlgItem(h, companion as i32), &mut feedback) != 0 {
                MapWindowPoints(0, h, (&mut feedback as *mut Rect).cast(), 2);
                let top = control.top.min(feedback.top);
                let bottom = control.bottom.max(feedback.bottom);
                if bottom - top <= client.bottom - 2 * margin {
                    control.top = top;
                    control.bottom = bottom;
                }
            }
        }
        if control.top < margin {
            scroll_to(h, SCROLL_OFFSET + control.top - margin);
        } else if control.bottom > client.bottom - margin {
            scroll_to(h, SCROLL_OFFSET + control.bottom - client.bottom + margin);
        }
    }
    unsafe fn dip_rect(x: i32, y: i32, w: i32, height: i32, scroll: bool) -> Rect {
        let offset = if scroll { SCROLL_OFFSET } else { 0 };
        Rect {
            left: scale_for_dpi(PANEL_WIDTH.x(x), UI_DPI),
            top: scale_for_dpi(y, UI_DPI) - offset,
            right: scale_for_dpi(PANEL_WIDTH.x(x + w), UI_DPI),
            bottom: scale_for_dpi(y + height, UI_DPI) - offset,
        }
    }
    unsafe fn rounded(dc: Handle, rect: &Rect, radius: i32, fill: u32, border: u32) {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(0, scale_for_dpi(1, UI_DPI), border);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            scale_for_dpi(radius, UI_DPI),
            scale_for_dpi(radius, UI_DPI),
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
    unsafe fn focus_outline(dc: Handle, rect: &Rect, radius: i32, color: u32) {
        let pen = CreatePen(0, scale_for_dpi(2, UI_DPI), color);
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, GetStockObject(5));
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            scale_for_dpi(radius, UI_DPI),
            scale_for_dpi(radius, UI_DPI),
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(pen);
    }
    unsafe fn paint_label(
        dc: Handle,
        label: &str,
        mut bounds: Rect,
        font: Handle,
        color: u32,
        flags: u32,
    ) {
        let old = SelectObject(dc, font);
        SetBkMode(dc, 1);
        SetTextColor(dc, color);
        DrawTextW(dc, wide(label).as_ptr(), -1, &mut bounds, flags | 0x800);
        SelectObject(dc, old);
    }
    unsafe fn paint_background(h: Handle, dc: Handle) {
        let mut client: Rect = zeroed();
        GetClientRect(h, &mut client);
        FillRect(dc, &client, BACKGROUND_BRUSH);
        let rail = Rect {
            left: 0,
            top: 0,
            right: scale_for_dpi(PANEL_WIDTH.sidebar, UI_DPI),
            bottom: client.bottom,
        };
        FillRect(dc, &rail, SIDEBAR_BRUSH);
        DrawIconEx(
            dc,
            scale_for_dpi(PANEL_WIDTH.x(20), UI_DPI),
            scale_for_dpi(28, UI_DPI),
            APP_ICON,
            scale_for_dpi(28, UI_DPI),
            scale_for_dpi(28, UI_DPI),
            0,
            0,
            3,
        );
        paint_label(
            dc,
            "校园网守护",
            dip_rect(56, 26, 104, 24, false),
            SECTION_FONT,
            theme::TEXT_PRIMARY,
            0,
        );
        paint_label(
            dc,
            "武汉理工大学",
            dip_rect(56, 52, 104, 20, false),
            SMALL_FONT,
            theme::TEXT_TERTIARY,
            0,
        );
        if ACTIVE_PAGE == 1 {
            let shift = if viewport_height(h) < COMPACT_OVERVIEW_HEIGHT {
                COMPACT_OVERVIEW_SHIFT
            } else {
                0
            };
            rounded(
                dc,
                &dip_rect(192, 96 - shift / 2, 544, 144 + HERO_EXTRA - shift / 2, true),
                12,
                theme::HERO,
                theme::HERO,
            );
            let y = DIAGNOSTICS_TOP + HERO_EXTRA - shift;
            let network_height = 76 + NETWORK_TEXT_HEIGHT + NETWORK_DETAIL_HEIGHT;
            for (top, height) in [
                (y, DIAGNOSTICS_HEIGHT),
                (y + DIAGNOSTICS_HEIGHT + 16, network_height),
            ] {
                rounded(
                    dc,
                    &dip_rect(192, top, 544, height, true),
                    12,
                    theme::SURFACE,
                    theme::SURFACE_BORDER,
                );
            }
        } else {
            for (y, height) in [(88, 96), (200, 152), (368, 152)] {
                let shift = form_feedback_shift(y)
                    + if y >= 200 { FORM_NETWORK_EXTRA } else { 0 }
                    + if y >= 368 { FORM_WRAP.account } else { 0 };
                let extra = match y {
                    88 => FORM_NETWORK_EXTRA,
                    200 => FORM_WRAP.account,
                    _ => FORM_WRAP.automation,
                };
                rounded(
                    dc,
                    &dip_rect(192, y + shift, 544, height + extra, true),
                    12,
                    theme::SURFACE,
                    theme::SURFACE_BORDER,
                );
            }
            rounded(
                dc,
                &dip_rect(
                    192,
                    ANALYTICS_TOP + settings_analytics_shift(),
                    544,
                    analytics_card_height(),
                    true,
                ),
                12,
                theme::SURFACE,
                theme::SURFACE_BORDER,
            );
            for (id, x, y, width) in [
                (C_USER, 212, 272, 240),
                (C_PASS, 476, 272, 240),
                (C_ADAPTER, 212, 158, 240),
                (C_SECONDARY_ADAPTER, 476, 158, 240),
            ] {
                let shift = form_feedback_shift(y) + if y >= 200 { FORM_NETWORK_EXTRA } else { 0 };
                let bounds = dip_rect(x, y + shift, width, 40, true);
                let error = FORM_ERROR_FIELD == id;
                rounded(
                    dc,
                    &bounds,
                    8,
                    theme::SURFACE,
                    if error {
                        theme::DANGER_TEXT
                    } else {
                        theme::CONTROL_BORDER
                    },
                );
                if GetFocus() == GetDlgItem(h, id as i32) {
                    focus_outline(dc, &bounds, 8, theme::FOCUS);
                }
            }
        }
    }
    unsafe fn draw_footer_icon(dc: Handle, bounds: &Rect, update: bool) {
        let (tile, ink) = if update {
            (theme::PRIMARY_SUBTLE, theme::PRIMARY)
        } else {
            (theme::NEUTRAL_HOVER, theme::TEXT_SECONDARY)
        };
        rounded(dc, bounds, 8, tile, tile);
        let icon = scale_for_dpi(16, UI_DPI);
        let left = (bounds.left + bounds.right - icon) / 2;
        let top = (bounds.top + bounds.bottom - icon) / 2;
        let x = |offset| left + scale_for_dpi(offset, UI_DPI);
        let y = |offset| top + scale_for_dpi(offset, UI_DPI);
        let pen = CreatePen(0, scale_for_dpi(2, UI_DPI), ink);
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, GetStockObject(5));
        if update {
            GdiArc(dc, x(2), y(2), x(15), y(15), x(13), y(4), x(15), y(10));
            MoveToEx(dc, x(14), y(1), null_mut());
            LineTo(dc, x(14), y(5));
            LineTo(dc, x(10), y(5));
        } else {
            Ellipse(dc, x(5), y(2), x(11), y(8));
            GdiArc(dc, x(2), y(9), x(14), y(20), x(14), y(14), x(2), y(14));
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(pen);
    }
    unsafe fn draw_footer_card(item: &DrawItem, color: u32) {
        let bounds = &item.rect;
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        let px = |dip| scale_for_dpi(dip, UI_DPI);
        let split = height < px(28);
        let stacked_text = height >= px(44);
        let update = item.control_id as usize == C_UPDATE;
        let state = app()
            .and_then(|app| app.update_state.get())
            .unwrap_or(UpdateState::Idle);
        let (compact_title, detail) = update_caption(state);
        let inset = px(if split { 3 } else { 10 });
        let mut left = bounds.left + inset;
        if !split && width >= px(if stacked_text { 128 } else { 116 }) {
            let tile = px(if stacked_text { 28 } else { 22 });
            let top = bounds.top + (height - tile) / 2;
            draw_footer_icon(
                item.dc,
                &Rect {
                    left,
                    top,
                    right: left + tile,
                    bottom: top + tile,
                },
                update,
            );
            left += tile + px(8);
        }
        let right = bounds.right - inset;
        let title = if update && stacked_text {
            "检查更新"
        } else if update && split && state == UpdateState::Current {
            "已是最新"
        } else if update {
            compact_title
        } else if split {
            "作者"
        } else {
            "Liunian520"
        };
        let flags = 0x4 | 0x20 | 0x8000 | if split { 1 } else { 0 };
        paint_label(
            item.dc,
            title,
            Rect {
                left,
                top: bounds.top + if stacked_text { px(5) } else { 0 },
                right,
                bottom: if stacked_text {
                    bounds.top + px(26)
                } else {
                    bounds.bottom
                },
            },
            if stacked_text {
                SECTION_FONT
            } else {
                SMALL_FONT
            },
            color,
            flags,
        );
        if stacked_text {
            let version;
            let subtitle = if !update {
                "GitHub 主页"
            } else if detail.is_empty() {
                version = format!("版本 v{}", application_version());
                &version
            } else {
                detail
            };
            paint_label(
                item.dc,
                subtitle,
                Rect {
                    left,
                    top: bounds.top + px(27),
                    right,
                    bottom: bounds.bottom - px(4),
                },
                SMALL_FONT,
                theme::TEXT_TERTIARY,
                flags,
            );
        }
    }
    unsafe fn draw_button(item: &DrawItem) {
        let id = item.control_id as usize;
        let navigation = matches!(id, C_NAV_OVERVIEW | C_NAV_SETTINGS);
        let card = matches!(id, C_AUTHOR | C_UPDATE);
        let footer = card || id == C_ABOUT;
        let selected = (id == C_NAV_OVERVIEW && ACTIVE_PAGE == 1)
            || (id == C_NAV_SETTINGS && ACTIVE_PAGE == 2);
        let primary = matches!(id, C_SAVE | M_CHECK);
        let tertiary = id == C_DELETE;
        let hover = HOVERED_BUTTON == item.item;
        let pressed = item.state & 1 != 0;
        let disabled = item.state & 4 != 0;
        FillRect(
            item.dc,
            &item.rect,
            if navigation || footer {
                SIDEBAR_BRUSH
            } else if matches!(id, C_DIAGNOSTICS_HISTORY | C_DIAGNOSTICS_COPY) {
                CARD_BRUSH
            } else {
                BACKGROUND_BRUSH
            },
        );
        let fill = if disabled {
            theme::DISABLED_SURFACE
        } else if card {
            if pressed {
                theme::NEUTRAL_PRESSED
            } else if hover {
                theme::PRIMARY_SUBTLE
            } else {
                theme::SURFACE
            }
        } else if navigation || footer {
            if pressed {
                theme::NAV_PRESSED
            } else if selected {
                theme::PRIMARY_SUBTLE
            } else if hover {
                theme::NAV_HOVER
            } else {
                theme::SIDEBAR
            }
        } else if primary {
            if pressed {
                theme::PRIMARY_PRESSED
            } else if hover {
                theme::PRIMARY_HOVER
            } else {
                theme::PRIMARY
            }
        } else if tertiary && !hover && !pressed {
            theme::BACKGROUND
        } else if pressed {
            theme::NEUTRAL_PRESSED
        } else if hover {
            theme::NEUTRAL_HOVER
        } else {
            theme::SURFACE
        };
        let border = if card {
            theme::SURFACE_BORDER
        } else if primary || navigation || footer || tertiary {
            fill
        } else {
            theme::CONTROL_BORDER
        };
        rounded(item.dc, &item.rect, 8, fill, border);
        let color = if disabled {
            theme::DISABLED_TEXT
        } else if primary {
            theme::TEXT_ON_PRIMARY
        } else if card {
            theme::TEXT_PRIMARY
        } else if id == C_ABOUT {
            if pressed {
                theme::TEXT_SECONDARY
            } else {
                theme::TEXT_TERTIARY
            }
        } else if id == C_DELETE {
            if hover {
                theme::DANGER_TEXT
            } else {
                theme::TEXT_TERTIARY
            }
        } else if selected {
            if pressed {
                theme::NAV_SELECTED_PRESSED_TEXT
            } else {
                theme::NAV_SELECTED_TEXT
            }
        } else {
            theme::TEXT_SECONDARY
        };
        let mut bounds = Rect {
            left: item.rect.left,
            top: item.rect.top,
            right: item.rect.right,
            bottom: item.rect.bottom,
        };
        if selected {
            // Shape reinforces the current page; selection is not color alone.
            let indicator = Rect {
                left: item.rect.left + scale_for_dpi(3, UI_DPI),
                top: item.rect.top + scale_for_dpi(10, UI_DPI),
                right: item.rect.left + scale_for_dpi(6, UI_DPI),
                bottom: item.rect.bottom - scale_for_dpi(10, UI_DPI),
            };
            rounded(item.dc, &indicator, 3, theme::PRIMARY, theme::PRIMARY);
        }
        if navigation {
            let icon_x = item.rect.left + scale_for_dpi(18, UI_DPI);
            let icon_y = item.rect.top
                + if item.rect.bottom - item.rect.top < scale_for_dpi(40, UI_DPI) {
                    scale_for_dpi(8, UI_DPI)
                } else {
                    scale_for_dpi(14, UI_DPI)
                };
            let pen = CreatePen(0, scale_for_dpi(2, UI_DPI), color);
            let old_pen = SelectObject(item.dc, pen);
            let old_brush = SelectObject(item.dc, GetStockObject(5));
            if id == C_NAV_OVERVIEW {
                Ellipse(
                    item.dc,
                    icon_x,
                    icon_y,
                    icon_x + scale_for_dpi(16, UI_DPI),
                    icon_y + scale_for_dpi(16, UI_DPI),
                );
                MoveToEx(
                    item.dc,
                    icon_x + scale_for_dpi(4, UI_DPI),
                    icon_y + scale_for_dpi(9, UI_DPI),
                    null_mut(),
                );
                LineTo(
                    item.dc,
                    icon_x + scale_for_dpi(7, UI_DPI),
                    icon_y + scale_for_dpi(12, UI_DPI),
                );
                LineTo(
                    item.dc,
                    icon_x + scale_for_dpi(13, UI_DPI),
                    icon_y + scale_for_dpi(5, UI_DPI),
                );
            } else {
                RoundRect(
                    item.dc,
                    icon_x,
                    icon_y,
                    icon_x + scale_for_dpi(16, UI_DPI),
                    icon_y + scale_for_dpi(16, UI_DPI),
                    scale_for_dpi(4, UI_DPI),
                    scale_for_dpi(4, UI_DPI),
                );
                MoveToEx(
                    item.dc,
                    icon_x + scale_for_dpi(5, UI_DPI),
                    icon_y + scale_for_dpi(5, UI_DPI),
                    null_mut(),
                );
                LineTo(
                    item.dc,
                    icon_x + scale_for_dpi(11, UI_DPI),
                    icon_y + scale_for_dpi(5, UI_DPI),
                );
                MoveToEx(
                    item.dc,
                    icon_x + scale_for_dpi(5, UI_DPI),
                    icon_y + scale_for_dpi(11, UI_DPI),
                    null_mut(),
                );
                LineTo(
                    item.dc,
                    icon_x + scale_for_dpi(11, UI_DPI),
                    icon_y + scale_for_dpi(11, UI_DPI),
                );
            }
            SelectObject(item.dc, old_pen);
            SelectObject(item.dc, old_brush);
            DeleteObject(pen);
            bounds.left += scale_for_dpi(48, UI_DPI);
        }
        if card {
            draw_footer_card(item, color);
        } else {
            let mut buf = [0u16; 128];
            let n = GetWindowTextW(item.item, buf.as_mut_ptr(), buf.len() as i32);
            let native_label = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            let label = match id {
                C_NAV_OVERVIEW => "连接概览",
                C_NAV_SETTINGS => "认证设置",
                _ => &native_label,
            };
            if footer {
                bounds.left += scale_for_dpi(12, UI_DPI);
            }
            paint_label(
                item.dc,
                label,
                bounds,
                if id == C_ABOUT { SMALL_FONT } else { UI_FONT },
                color,
                0x4 | 0x20 | if navigation || footer { 0 } else { 0x1 },
            );
        }
        if item.state & 0x10 != 0 && item.state & 0x200 == 0 {
            let inset = scale_for_dpi(3, UI_DPI);
            let focus = Rect {
                left: item.rect.left + inset,
                top: item.rect.top + inset,
                right: item.rect.right - inset,
                bottom: item.rect.bottom - inset,
            };
            focus_outline(
                item.dc,
                &focus,
                6,
                if primary {
                    theme::FOCUS_ON_PRIMARY
                } else {
                    theme::FOCUS
                },
            );
        }
    }
    unsafe extern "system" fn wndproc(h: Handle, m: u32, w: usize, l: isize) -> isize {
        match m {
            WM_SIZE => {
                if ACTIVE_PAGE == 1 && app().is_some() {
                    update_overview_layout(h);
                } else {
                    configure_scroll(h);
                    layout_controls(h);
                }
                0
            }
            WM_VSCROLL => {
                let bar = GetDlgItem(h, C_PAGE_SCROLL as i32);
                if bar == 0 || (l != 0 && l != bar) {
                    return DefWindowProcW(h, m, w, l);
                }
                let mut info = ScrollInfo {
                    size: size_of::<ScrollInfo>() as u32,
                    mask: 0x2 | 0x10,
                    min: 0,
                    max: 0,
                    page: 0,
                    position: 0,
                    track_position: 0,
                };
                GetScrollInfo(bar, SB_CTL, &mut info);
                let line = scale_for_dpi(40, UI_DPI);
                let page = (info.page as i32 - line).max(line);
                let next = match w & 0xffff {
                    0 => SCROLL_OFFSET - line,
                    1 => SCROLL_OFFSET + line,
                    2 => SCROLL_OFFSET - page,
                    3 => SCROLL_OFFSET + page,
                    4 => (w >> 16) as u16 as i32,
                    5 => info.track_position,
                    6 => 0,
                    7 => scale_for_dpi(content_height(h), UI_DPI),
                    _ => SCROLL_OFFSET,
                };
                scroll_from_input(h, next);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((w >> 16) as u16 as i16) as i32;
                scroll_from_input(h, SCROLL_OFFSET - delta * scale_for_dpi(120, UI_DPI) / 120);
                0
            }
            WM_SETTINGCHANGE => {
                let always_visible = always_show_page_scrollbar();
                if SCROLL_ALWAYS_VISIBLE != always_visible {
                    SCROLL_ALWAYS_VISIBLE = always_visible;
                    let bar = GetDlgItem(h, C_PAGE_SCROLL as i32);
                    if bar != 0 {
                        InvalidateRect(bar, null(), 0);
                    }
                }
                DefWindowProcW(h, m, w, l)
            }
            WM_ERASEBKGND if !HIGH_CONTRAST => {
                paint_background(h, w as Handle);
                1
            }
            WM_DRAWITEM if !HIGH_CONTRAST && l != 0 => {
                let item = &*(l as *const DrawItem);
                if matches!(
                    item.control_id as usize,
                    C_SAVE
                        | C_DELETE
                        | C_NAV_OVERVIEW
                        | C_NAV_SETTINGS
                        | C_AUTHOR
                        | C_UPDATE
                        | C_ABOUT
                        | M_CHECK
                        | M_OPEN
                        | M_CHECK_EXTERNAL
                        | C_DIAGNOSTICS_HISTORY
                        | C_DIAGNOSTICS_COPY
                ) {
                    draw_button(item);
                    1
                } else {
                    DefWindowProcW(h, m, w, l)
                }
            }
            WM_CTLCOLORSTATIC if !HIGH_CONTRAST => {
                let id = GetDlgCtrlID(l as Handle) as usize;
                let hero = matches!(id, C_STATUS | C_STATUS_LABEL | C_STATUS_HEAD);
                let muted = matches!(
                    id,
                    C_SUBTITLE
                        | C_HINT_NETWORK
                        | C_HINT_PASSWORD
                        | C_FOOTER
                        | C_START_HINT
                        | C_FORM_SUBTITLE
                        | C_NETWORK_DESC
                        | C_OVERVIEW_HINT
                        | C_NETWORK_LABEL
                        | C_HTTP_HINT
                        | C_ADAPTER_DETAIL
                        | C_ANALYTICS_DISCLOSURE
                        | C_DIAGNOSTICS_FEEDBACK
                );
                let on_background = matches!(
                    id,
                    C_TITLE
                        | C_SUBTITLE
                        | C_SETTINGS_TITLE
                        | C_FORM_SUBTITLE
                        | C_FOOTER
                        | C_OVERVIEW_HINT
                        | C_FORM_FEEDBACK
                );
                SetTextColor(
                    w as Handle,
                    if id == C_FORM_FEEDBACK && FORM_ERROR_FIELD != 0 {
                        theme::DANGER_TEXT
                    } else if hero {
                        if id == C_STATUS_HEAD {
                            theme::TEXT_ON_HERO
                        } else {
                            theme::TEXT_HERO_SECONDARY
                        }
                    } else if muted {
                        theme::TEXT_TERTIARY
                    } else {
                        theme::TEXT_PRIMARY
                    },
                );
                SetBkColor(
                    w as Handle,
                    if hero {
                        theme::HERO
                    } else if on_background {
                        theme::BACKGROUND
                    } else {
                        theme::SURFACE
                    },
                );
                if hero {
                    HERO_BRUSH
                } else if on_background {
                    BACKGROUND_BRUSH
                } else {
                    CARD_BRUSH
                }
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX if !HIGH_CONTRAST => {
                SetTextColor(w as Handle, theme::TEXT_PRIMARY);
                SetBkColor(w as Handle, theme::SURFACE);
                CARD_BRUSH
            }
            WM_CTLCOLORBTN if !HIGH_CONTRAST => {
                SetTextColor(w as Handle, theme::TEXT_PRIMARY);
                SetBkColor(w as Handle, theme::SURFACE);
                CARD_BRUSH
            }
            WM_CLOSE => {
                SetWindowTextW(GetDlgItem(h, C_PASS as i32), wide("").as_ptr());
                if app().is_some_and(|state| state.tray_available.get()) {
                    ShowWindow(h, 0);
                } else {
                    DestroyWindow(h);
                }
                0
            }
            WM_DESTROY => {
                super::update_notice::close();
                KillTimer(h, 1);
                if let Some(app) = app() {
                    if app.tray_available.get() {
                        Shell_NotifyIconW(NIM_DELETE, &mut *app.tray.borrow_mut());
                    }
                    notify_quit(&app.generation, &app.gate, &app.tx);
                }
                PostQuitMessage(0);
                0
            }
            WM_TIMER => {
                if w == 1 {
                    refresh_status();
                    refresh_update_card(h);
                    if ACTIVE_PAGE == 1 && IsWindowVisible(h) != 0 {
                        if let Some(app) = app() {
                            if refresh_diagnostics(h, app) {
                                update_overview_layout(h);
                            }
                        }
                    }
                }
                0
            }
            WM_POWERBROADCAST => {
                if w == 4 || w == 18 || w == 7 {
                    if let Some(app) = app() {
                        if notify_network_change(&app.generation, &app.gate, &app.tx).is_err() {
                            let _ = app.tx.send(Event::SettingsFailed);
                        }
                    }
                }
                1
            }
            message
                if app().is_some_and(|state| {
                    state.taskbar_created != 0 && state.taskbar_created == message
                }) =>
            {
                if let Some(app) = app() {
                    let available = register_tray(&mut *app.tray.borrow_mut());
                    app.tray_available.set(available);
                    SetWindowTextW(
                        GetDlgItem(h, C_FOOTER as i32),
                        wide(overview_footer(available)).as_ptr(),
                    );
                    if !available && IsWindowVisible(h) == 0 {
                        // The panel is the remaining entry point. Do not steal focus.
                        ShowWindow(h, 4);
                    }
                }
                0
            }
            TRAY => {
                match tray_action(w, l) {
                    Some(TrayAction::Open) => open_panel(h, 1),
                    Some(TrayAction::Menu { use_icon }) => menu(h, use_icon),
                    None => {}
                }
                0
            }
            WM_COMMAND => {
                let id = w & 0xffff;
                let notification = w >> 16;
                let adapter_changed =
                    matches!(id, C_ADAPTER | C_SECONDARY_ADAPTER) && notification == 1;
                // CBN_DROPDOWN is sent before the list appears. Refresh options
                // after a cable or Wi-Fi change without resetting draft selections.
                if matches!(id, C_ADAPTER | C_SECONDARY_ADAPTER) && notification == 7 {
                    refresh_adapters(h);
                }
                if adapter_changed {
                    update_adapter_detail(h);
                }
                if FORM_ERROR_FIELD != 0
                    && ((id == FORM_ERROR_FIELD
                        && ((matches!(id, C_USER | C_PASS) && notification == 0x300)
                            || adapter_changed
                            || (id == C_CONSENT && notification == 0)))
                        || (adapter_changed && FORM_ERROR_FIELD == C_SECONDARY_ADAPTER))
                {
                    form_feedback(h, "", 0);
                }
                match id {
                    C_NAV_OVERVIEW => show_page(h, 1),
                    M_TRAY_PANEL => open_panel(h, 1),
                    C_NAV_SETTINGS => show_page(h, 2),
                    C_AUTHOR => {
                        if ShellExecuteW(
                            h,
                            wide("open").as_ptr(),
                            wide(AUTHOR_URL).as_ptr(),
                            null(),
                            null(),
                            1,
                        ) <= 32
                        {
                            alert(h, "无法打开浏览器。作者主页：https://github.com/Liunian520");
                        }
                    }
                    C_UPDATE if notification == 0 => {
                        request_notice_check();
                        refresh_update_card(h);
                        super::update_notice::open(
                            h,
                            UI_DPI,
                            notice_snapshot,
                            request_notice_check,
                        );
                    }
                    C_ABOUT => show_about(h),
                    C_ANALYTICS if notification == 0 => change_analytics_consent(h),
                    C_SAVE => save_form(h),
                    C_DELETE => {
                        let Some(app) = app() else {
                            return 0;
                        };
                        if let Err(e) = invalidate_inflight(app) {
                            report_settings_failure(h, app, &e);
                            return 0;
                        }
                        match delete_password() {
                            Ok(()) => {
                                let _ = app.tx.send(Event::CredentialsDeleted);
                                alert(h, "已删除密码。自动登录将暂停，直到重新保存凭据。");
                            }
                            Err(e) => report_settings_failure(h, app, &e),
                        }
                    }
                    M_CHECK => {
                        if let Some(app) = app() {
                            let _ = app.tx.send(Event::CheckNow);
                        }
                    }
                    M_CHECK_EXTERNAL if notification == 0 => {
                        if let Some(app) = app() {
                            (app.diagnostics.check_external)();
                            if refresh_diagnostics(h, app) {
                                update_overview_layout(h);
                            }
                        }
                    }
                    C_DIAGNOSTICS_HISTORY if notification == 0 => {
                        if let Some(app) = app() {
                            let open = !app.history_open.get();
                            app.history_open.set(open);
                            let label = if open {
                                "隐藏恢复记录"
                            } else {
                                "查看恢复记录"
                            };
                            SetWindowTextW(
                                GetDlgItem(h, C_DIAGNOSTICS_HISTORY as i32),
                                wide(label).as_ptr(),
                            );
                            refresh_diagnostics(h, app);
                            update_overview_layout(h);
                            ensure_focus_visible(h);
                        }
                    }
                    C_DIAGNOSTICS_COPY if notification == 0 => {
                        if let Some(app) = app() {
                            copy_diagnostics(h, app);
                        }
                    }
                    M_SETTINGS => {
                        open_panel(h, 2);
                    }
                    M_ANALYTICS => {
                        open_panel(h, 2);
                        scroll_to(h, i32::MAX);
                        SetFocus(GetDlgItem(h, C_ANALYTICS as i32));
                    }
                    M_OPEN => {
                        ShellExecuteW(
                            h,
                            wide("open").as_ptr(),
                            wide(PORTAL).as_ptr(),
                            null(),
                            null(),
                            1,
                        );
                    }
                    M_QUIT => {
                        DestroyWindow(h);
                    }
                    _ => {}
                }
                0
            }
            _ => DefWindowProcW(h, m, w, l),
        }
    }
    pub fn run(
        initial: Settings,
        adapter_source: Box<dyn Fn() -> Vec<Adapter>>,
        tx: Sender<Event>,
        status: Arc<Mutex<String>>,
        generation: Arc<AtomicU64>,
        gate: Arc<Mutex<()>>,
        update: UiUpdates,
        analytics: UiAnalytics,
        diagnostics: UiDiagnostics,
    ) -> Result<(), String> {
        unsafe {
            SetProcessDPIAware();
            UI_DPI = GetDpiForSystem().max(96);
            #[cfg(debug_assertions)]
            if let Ok(value) = std::env::var("CAMPUS_UI_PREVIEW_DPI") {
                if let Ok(dpi) = value.parse::<u32>() {
                    if (96..=384).contains(&dpi) {
                        UI_DPI = dpi;
                    }
                }
            }
            let mut contrast = HighContrast {
                size: size_of::<HighContrast>() as u32,
                flags: 0,
                scheme: null_mut(),
            };
            HIGH_CONTRAST = SystemParametersInfoW(
                0x42,
                contrast.size,
                (&mut contrast as *mut HighContrast).cast(),
                0,
            ) != 0
                && contrast.flags & 1 != 0;
            SCROLL_ALWAYS_VISIBLE = always_show_page_scrollbar();
            BACKGROUND_BRUSH = CreateSolidBrush(theme::BACKGROUND);
            HERO_BRUSH = CreateSolidBrush(theme::HERO);
            SIDEBAR_BRUSH = CreateSolidBrush(theme::SIDEBAR);
            CARD_BRUSH = CreateSolidBrush(theme::SURFACE);
            let instance = GetModuleHandleW(null());
            let class = wide("CampusNetworkGuardianWindow");
            let icon = LoadIconW(instance, 1usize as *const u16);
            if icon == 0 {
                return Err(error("加载应用图标失败"));
            }
            APP_ICON = icon;
            let wc = WndClass {
                style: 0,
                proc: Some(wndproc),
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon,
                cursor: LoadCursorW(0, 32512usize as *const u16),
                background: 6,
                menu: null(),
                class: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                return Err(error("注册窗口失败"));
            }
            let mut work_area = Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if SystemParametersInfoW(0x30, 0, (&mut work_area as *mut Rect).cast(), 0) == 0 {
                work_area.right = scale_for_dpi(800, UI_DPI);
                work_area.bottom = scale_for_dpi(700, UI_DPI);
            }
            #[cfg(debug_assertions)]
            if let Some(width) = std::env::var("CAMPUS_UI_PREVIEW_WORK_WIDTH")
                .ok()
                .and_then(|value| value.parse::<i32>().ok())
                .filter(|width| *width > 0)
            {
                work_area.right = work_area.left + width;
            }
            let work_width = work_area.right - work_area.left;
            let work_height = work_area.bottom - work_area.top;
            let window_style = 0x00ca0000;
            let standard_width = scale_for_dpi(PanelWidth::STANDARD.width, UI_DPI);
            let mut frame = Rect {
                left: 0,
                top: 0,
                right: standard_width,
                bottom: scale_for_dpi(CONTENT_HEIGHT, UI_DPI),
            };
            AdjustWindowRectEx(&mut frame, window_style, 0, 0);
            let chrome_width = frame.right - frame.left - standard_width;
            PANEL_WIDTH = PanelWidth::fit(work_width, chrome_width, UI_DPI);
            let window_width = scale_for_dpi(PANEL_WIDTH.width, UI_DPI) + chrome_width;
            let window_height = (frame.bottom - frame.top).min(work_height);
            let h = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("校园网守护").as_ptr(),
                window_style,
                work_area.left + (work_width - window_width).max(0) / 2,
                work_area.top + (work_height - window_height).max(0) / 2,
                window_width,
                window_height,
                0,
                0,
                instance,
                null(),
            );
            if h == 0 {
                return Err(error("创建窗口失败"));
            }
            let corner: u32 = 2;
            let border: u32 = theme::WINDOW_BORDER;
            let _ = DwmSetWindowAttribute(h, 33, (&corner as *const u32).cast(), 4);
            let _ = DwmSetWindowAttribute(h, 34, (&border as *const u32).cast(), 4);
            UI_FONT = make_font(14, 400);
            TITLE_FONT = make_font(theme::PAGE_TITLE_SIZE, 600);
            SECTION_FONT = make_font(15, 600);
            SMALL_FONT = make_font(13, 400);
            HERO_FONT = make_font(theme::CONNECTION_TITLE_SIZE, 600);
            let button_style = if HIGH_CONTRAST { 0 } else { 0xb };
            BUILD_PAGE = 0;
            control(
                h,
                "BUTTON",
                "连接概览",
                WS_TABSTOP | button_style,
                12,
                104,
                144,
                40,
                C_NAV_OVERVIEW,
            );
            control(
                h,
                "BUTTON",
                "认证设置",
                WS_TABSTOP | button_style,
                12,
                152,
                144,
                40,
                C_NAV_SETTINGS,
            );
            control(
                h,
                "BUTTON",
                "Liunian520\r\n作者 · GitHub 主页",
                WS_TABSTOP | 0x2000 | button_style,
                12,
                448,
                144,
                52,
                C_AUTHOR,
            );
            control(
                h,
                "BUTTON",
                "检查更新",
                WS_TABSTOP | 0x2000 | button_style,
                12,
                508,
                144,
                52,
                C_UPDATE,
            );
            control(
                h,
                "BUTTON",
                "关于 · MIT",
                WS_TABSTOP | button_style,
                12,
                568,
                144,
                24,
                C_ABOUT,
            );
            BUILD_PAGE = 1;
            control(h, "STATIC", "连接概览", 0, 192, 24, 544, 32, C_TITLE);
            control(
                h,
                "STATIC",
                "连接状态与当前校园网络",
                0,
                192,
                60,
                544,
                20,
                C_SUBTITLE,
            );
            control(
                h,
                "STATIC",
                "当前连接",
                0,
                216,
                112,
                496,
                20,
                C_STATUS_LABEL,
            );
            control(
                h,
                "STATIC",
                "正在确认连接",
                0,
                216,
                136,
                496,
                34,
                C_STATUS_HEAD,
            );
            control(
                h,
                "EDIT",
                "正在启动",
                WS_TABSTOP | 0x4 | 0x40 | 0x800 | 0x200000,
                216,
                180,
                496,
                44,
                C_STATUS,
            );
            control(
                h,
                "BUTTON",
                "立即检查",
                WS_TABSTOP | button_style,
                192,
                256,
                144,
                40,
                M_CHECK,
            );
            control(
                h,
                "BUTTON",
                "打开认证页面",
                WS_TABSTOP | button_style,
                352,
                256,
                176,
                40,
                M_OPEN,
            );
            control(
                h,
                "BUTTON",
                "检测外网",
                WS_TABSTOP | button_style,
                544,
                256,
                152,
                40,
                M_CHECK_EXTERNAL,
            );
            control(
                h,
                "STATIC",
                "连接诊断",
                0,
                216,
                338,
                496,
                24,
                C_DIAGNOSTICS_TITLE,
            );
            control(
                h,
                "STATIC",
                "等待首次诊断。",
                0,
                216,
                368,
                496,
                20,
                C_DIAGNOSTICS_SUMMARY,
            );
            control(
                h,
                "BUTTON",
                "查看恢复记录",
                WS_TABSTOP | button_style,
                216,
                400,
                148,
                36,
                C_DIAGNOSTICS_HISTORY,
            );
            control(
                h,
                "BUTTON",
                "复制诊断",
                WS_TABSTOP | button_style,
                376,
                400,
                116,
                36,
                C_DIAGNOSTICS_COPY,
            );
            control(h, "STATIC", "", 0, 216, 446, 496, 0, C_DIAGNOSTICS_FEEDBACK);
            control(
                h,
                "EDIT",
                "",
                WS_TABSTOP | 0x4 | 0x40 | 0x800 | 0x200000 | 0x800000,
                216,
                446,
                496,
                RECOVERY_REPORT_HEIGHT,
                C_DIAGNOSTICS_REPORT,
            );
            control(
                h,
                "STATIC",
                "当前校园网络",
                0,
                216,
                340,
                496,
                20,
                C_NETWORK_LABEL,
            );
            control(
                h,
                "STATIC",
                "尚未选择校园网络",
                0,
                216,
                372,
                496,
                28,
                C_NETWORK_VALUE,
            );
            control(
                h,
                "STATIC",
                "仅在匹配所选网络时执行认证",
                0,
                216,
                404,
                496,
                20,
                C_NETWORK_DESC,
            );
            control(h, "STATIC", "", 0, 192, 460, 544, 0, C_OVERVIEW_HINT);
            control(
                h,
                "STATIC",
                overview_footer(true),
                0,
                192,
                468,
                544,
                44,
                C_FOOTER,
            );
            BUILD_PAGE = 2;
            control(
                h,
                "STATIC",
                "认证设置",
                0,
                192,
                24,
                544,
                32,
                C_SETTINGS_TITLE,
            );
            control(
                h,
                "STATIC",
                "管理校园网络、凭据与自动启动",
                0,
                192,
                60,
                544,
                20,
                C_FORM_SUBTITLE,
            );
            control(h, "STATIC", "", 0, 192, 88, 544, 0, C_FORM_FEEDBACK);
            control(h, "STATIC", "校园网络", 0, 212, 104, 90, 22, C_FORM_NETWORK);
            control(
                h,
                "STATIC",
                "无线先连校园 Wi-Fi，再选择网卡。\r\n自动关闭随机 MAC；必要时等待重连。",
                0,
                308,
                96,
                408,
                40,
                C_HINT_NETWORK,
            );
            for (x, label, label_id, combo_id) in [
                (212, "主网络", C_LABEL_ADAPTER, C_ADAPTER),
                (
                    476,
                    "第二个网络（可选）",
                    C_LABEL_SECONDARY_ADAPTER,
                    C_SECONDARY_ADAPTER,
                ),
            ] {
                control(h, "STATIC", label, 0, x, 134, 240, 20, label_id);
                control(
                    h,
                    "COMBOBOX",
                    "",
                    WS_TABSTOP | 0x3 | 0x200000,
                    x + 10,
                    164,
                    ADAPTER_COMBO_WIDTH,
                    210,
                    combo_id,
                );
                let combo = GetDlgItem(h, combo_id as i32);
                SendMessageW(combo, 0x153, usize::MAX, scale_for_dpi(23, UI_DPI) as isize);
                SendMessageW(
                    combo,
                    0x160,
                    scale_for_dpi(360.min(PANEL_WIDTH.width), UI_DPI) as usize,
                    0,
                );
            }
            control(h, "STATIC", "", 0, 212, 208, 504, 0, C_ADAPTER_DETAIL);
            control(
                h,
                "STATIC",
                "账号与密码",
                0,
                212,
                216,
                504,
                22,
                C_FORM_ACCOUNT,
            );
            control(
                h,
                "STATIC",
                "校园网账号",
                0,
                212,
                248,
                240,
                20,
                C_LABEL_USER,
            );
            control(h, "EDIT", "", WS_TABSTOP | 0x80, 224, 280, 216, 24, C_USER);
            control(h, "STATIC", "认证密码", 0, 476, 248, 240, 20, C_LABEL_PASS);
            control(
                h,
                "EDIT",
                "",
                WS_TABSTOP | 0x20 | 0x80,
                488,
                280,
                216,
                24,
                C_PASS,
            );
            SendMessageW(
                GetDlgItem(h, C_PASS as i32),
                0x1501,
                0,
                wide("留空保留原密码").as_ptr() as isize,
            );
            control(
                h,
                "STATIC",
                "所选网络共用账号和密码。密码保存在 Windows 凭据管理器中，留空时沿用。",
                0,
                212,
                320,
                504,
                20,
                C_HINT_PASSWORD,
            );
            control(
                h,
                "STATIC",
                "自动化",
                0,
                212,
                384,
                504,
                22,
                C_FORM_AUTOMATION,
            );
            control(
                h,
                "BUTTON",
                "我了解并同意通过 HTTP 认证所选网络",
                WS_TABSTOP | 3 | 0x2000,
                212,
                414,
                504,
                22,
                C_CONSENT,
            );
            control(
                h,
                "STATIC",
                "密码将以明文传输，门户身份无法验证。",
                0,
                234,
                440,
                482,
                20,
                C_HTTP_HINT,
            );
            control(
                h,
                "BUTTON",
                "登录 Windows 后自动启动",
                WS_TABSTOP | 3 | 0x2000,
                212,
                472,
                504,
                22,
                C_START,
            );
            control(
                h,
                "STATIC",
                "启动后在托盘静默守护，不打断当前操作。",
                0,
                234,
                496,
                482,
                18,
                C_START_HINT,
            );
            control(
                h,
                "BUTTON",
                if PANEL_WIDTH.width < 540 {
                    "保存设置"
                } else {
                    "保存并开始守护"
                },
                WS_TABSTOP | button_style,
                192,
                536,
                192,
                40,
                C_SAVE,
            );
            control(
                h,
                "BUTTON",
                if PANEL_WIDTH.width < 540 {
                    "删除密码"
                } else {
                    "删除保存的密码"
                },
                WS_TABSTOP | button_style,
                560,
                536,
                176,
                40,
                C_DELETE,
            );
            control(
                h,
                "BUTTON",
                "允许发送匿名使用统计",
                WS_TABSTOP | 3 | 0x2000,
                212,
                ANALYTICS_TOP + 16,
                504,
                28,
                C_ANALYTICS,
            );
            control(
                h,
                "STATIC",
                ANALYTICS_DISCLOSURE,
                0,
                234,
                ANALYTICS_DISCLOSURE_TOP,
                482,
                0,
                C_ANALYTICS_DISCLOSURE,
            );
            BUILD_PAGE = 0;
            measure_form_text(h);
            let first = initial.adapter_id.is_empty()
                || initial.username.is_empty()
                || !initial.http_acknowledged;
            // Put the native scrollbar in the page gutter and last in Tab order.
            let page_scrollbar = CreateWindowExW(
                0,
                wide("SCROLLBAR").as_ptr(),
                wide("页面滚动").as_ptr(),
                WS_CHILD | WS_TABSTOP | 1,
                0,
                0,
                0,
                0,
                h,
                C_PAGE_SCROLL as isize,
                instance,
                null(),
            );
            if page_scrollbar == 0 {
                DestroyWindow(h);
                return Err(error("创建页面滚动条失败"));
            }
            if !HIGH_CONTRAST {
                SetWindowSubclass(page_scrollbar, Some(page_scrollbar_proc), 1, 0);
            }
            let mut tray: Notify = zeroed();
            tray.size = size_of::<Notify>() as u32;
            tray.hwnd = h;
            tray.id = TRAY_ICON_ID;
            tray.callback = TRAY;
            tray.icon = icon;
            for (to, from) in tray.tip.iter_mut().zip(wide("校园网守护")) {
                *to = from;
            }
            // Keep a visible panel when the desktop shell cannot host a tray icon.
            let tray_available = register_tray(&mut tray);
            let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            const MSGFLT_ALLOW: u32 = 1;
            // Explorer runs with ordinary privileges, even when this app is elevated.
            // These callbacks only expose the window, menu or notification icon.
            ChangeWindowMessageFilterEx(h, TRAY, MSGFLT_ALLOW, null_mut());
            if taskbar_created != 0 {
                ChangeWindowMessageFilterEx(h, taskbar_created, MSGFLT_ALLOW, null_mut());
            }
            let state = Box::new(App {
                settings: RefCell::new(initial),
                adapters: RefCell::new(Vec::new()),
                adapter_source,
                tx,
                status,
                tray: RefCell::new(tray),
                tray_available: Cell::new(tray_available),
                taskbar_created,
                last_status: RefCell::new(String::new()),
                update,
                update_state: Cell::new(None),
                analytics,
                diagnostics,
                history_open: Cell::new(false),
                generation,
                gate,
            });
            APP = &*state;
            SetWindowTextW(
                GetDlgItem(h, C_FOOTER as i32),
                wide(overview_footer(tray_available)).as_ptr(),
            );
            reset_form(h);
            show_page(h, if first { 2 } else { 1 });
            SetTimer(h, 1, 1000, null());
            refresh_status();
            refresh_update_card(h);
            if first || !tray_available || std::env::args().any(|arg| arg == "--show") {
                open_panel(h, if first { 2 } else { 1 });
            }
            let mut msg: Msg = zeroed();
            let mut last_focus = 0;
            let result = loop {
                let r = GetMessageW(&mut msg, 0, 0, 0);
                if r <= 0 {
                    break if r < 0 {
                        Err(error("窗口消息循环失败"))
                    } else {
                        Ok(())
                    };
                }
                if !super::update_notice::dispatch_dialog((&mut msg as *mut Msg).cast())
                    && IsDialogMessageW(h, &mut msg) == 0
                {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let focus = GetFocus();
                if focus != last_focus || matches!(msg.message, 0x100 | 0x102) {
                    ensure_focus_visible(h);
                }
                last_focus = focus;
            };
            APP = null();
            if UI_FONT != 0 {
                DeleteObject(UI_FONT);
            }
            if TITLE_FONT != 0 {
                DeleteObject(TITLE_FONT);
            }
            if SECTION_FONT != 0 {
                DeleteObject(SECTION_FONT);
            }
            if BACKGROUND_BRUSH != 0 {
                DeleteObject(BACKGROUND_BRUSH);
            }
            if CARD_BRUSH != 0 {
                DeleteObject(CARD_BRUSH);
            }
            for resource in [SMALL_FONT, HERO_FONT, HERO_BRUSH, SIDEBAR_BRUSH] {
                if resource != 0 {
                    DeleteObject(resource);
                }
            }
            LAYOUT.with(|layout| layout.borrow_mut().clear());
            result
        }
    }
}
#[cfg(windows)]
pub use win::{autostart_enabled, read_password, run, show_existing_panel, single_instance};
