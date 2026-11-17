#[cfg(windows)]
#[path = "ui_theme.rs"]
mod theme;

#[cfg(test)]
mod tests {
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
        assert!(super::settings_allowed("adapter-1", "alice", true));
        assert!(!super::settings_allowed("adapter-1", "alice", false));
        assert!(!super::settings_allowed("", "alice", true));
        assert!(!super::settings_allowed("adapter-1", "", true));
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
    Attention,
    Pending,
}
fn connection_tone(status: &str) -> ConnectionTone {
    if status == "校园网在线" {
        ConnectionTone::Online
    } else if should_notify("", status)
        || status.contains("暂停")
        || status.contains("人工处理")
        || status.contains("密码有误")
        || status.contains("账号异常")
    {
        ConnectionTone::Attention
    } else {
        ConnectionTone::Pending
    }
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
    old != new
        && (new.contains("人工处理")
            || new.contains("自动操作已暂停")
            || new.contains("自动登录已暂停")
            || matches!(
                new,
                "登录预检提示密码有误，请检查并重新保存设置"
                    | "登录预检提示账号异常，请打开认证页面检查"
                    | "登录预检提示认证参数异常，请打开认证页面检查"
                    | "登录预检未通过，请打开认证页面检查"
                    | "门户提示密码有误，请检查并重新保存设置"
                    | "门户提示账号异常，请打开认证页面检查"
                    | "门户提示认证参数异常，请打开认证页面检查"
                    | "门户未接受登录，请打开认证页面检查"
            ))
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
fn settings_allowed(adapter: &str, username: &str, consent: bool) -> bool {
    !adapter.trim().is_empty() && !username.trim().is_empty() && consent
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

#[derive(Clone, Debug, Default)]
pub struct Adapter {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub adapter_id: String,
    pub username: String,
    pub http_acknowledged: bool,
    pub autostart: bool,
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
        autostart_value_matches, connection_tone, copy_utf16, matches_terminated_username,
        notify_network_change, notify_quit, scale_for_dpi, settings_allowed,
        settings_failure_message, settings_transaction, should_notify, Adapter, ConnectionTone,
        Event, Settings,
    };
    use std::{
        cell::RefCell,
        ffi::c_void,
        mem::{size_of, zeroed},
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc::{self, Sender},
            Arc, Mutex,
        },
    };
    type Handle = isize;
    const TARGET: &str = "CampusNetworkGuardian/Portal";
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const APP_NAME: &str = "CampusNetworkGuardian";
    const PORTAL: &str = "http://172.30.21.100/";
    const AUTHOR_URL: &str = "https://github.com/Liunian520";
    const TRAY: u32 = 0x8001;
    const WM_DESTROY: u32 = 2;
    const WM_CLOSE: u32 = 0x10;
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
    const WM_RBUTTONUP: u32 = 0x205;
    const WM_LBUTTONDBLCLK: u32 = 0x203;
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
    const C_NAV_OVERVIEW: usize = 301;
    const C_NAV_SETTINGS: usize = 302;
    const C_AUTHOR: usize = 303;
    const C_ABOUT: usize = 304;
    const M_CHECK: usize = 201;
    const M_SETTINGS: usize = 202;
    const M_OPEN: usize = 203;
    const M_QUIT: usize = 204;
    const M_ABOUT: usize = 205;
    const WS_CHILD: u32 = 0x40000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_TABSTOP: u32 = 0x10000;
    const CONTENT_HEIGHT: i32 = 600;
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
        timeout: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: Guid,
        balloon: Handle,
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
        fn SetFocus(h: Handle) -> Handle;
        fn GetParent(h: Handle) -> Handle;
        fn MapWindowPoints(from: Handle, to: Handle, points: *mut Point, count: u32) -> i32;
        fn GetScrollInfo(h: Handle, bar: i32, info: *mut ScrollInfo) -> i32;
        fn SetScrollInfo(h: Handle, bar: i32, info: *const ScrollInfo, redraw: i32) -> i32;
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
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(msg: u32, data: *mut Notify) -> i32;
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
                PostMessageW(window, TRAY, 1, WM_LBUTTONDBLCLK as isize);
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
        last_status: RefCell<String>,
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
    static mut ACTIVE_PAGE: u8 = 1;
    static mut BUILD_PAGE: u8 = 0;
    static mut HOVERED_BUTTON: Handle = 0;
    static mut HERO_EXTRA: i32 = 0;
    static mut NETWORK_TEXT_HEIGHT: i32 = 28;
    static mut OVERVIEW_HEIGHT: i32 = CONTENT_HEIGHT;
    static mut FORM_SHIFT: i32 = 0;
    static mut FORM_NETWORK_EXTRA: i32 = 0;
    static mut FORM_ERROR_FIELD: usize = 0;
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
        option_env!("CARGO_PKG_VERSION").unwrap_or("0.4.1")
    }
    unsafe fn show_about(h: Handle) {
        let message = format!(
            "校园网守护  v{}\r\n武汉理工大学校园网连接助手\r\n\r\n作者  Liunian520\r\n开源许可  MIT\r\n\r\n{}\r\n可点击侧栏作者署名，在浏览器中打开 GitHub 主页。",
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
            scale_for_dpi(x, UI_DPI),
            scale_for_dpi(y, UI_DPI),
            scale_for_dpi(w, UI_DPI),
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
                C_SECTION | C_FORM_NETWORK | C_FORM_ACCOUNT | C_FORM_AUTOMATION
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
        if m == 0x87 && GetDlgCtrlID(h) as usize == C_STATUS {
            // A multiline EDIT normally claims every key. Keep its arrow and
            // selection handling, but let dialog navigation handle Tab so the
            // read-only status remains accessible without trapping focus.
            return DefSubclassProc(h, m, w, l) & !(0x2 | 0x4);
        }
        // The combo remains a native keyboard- and accessibility-aware dropdown.
        // Only its closed surface is painted to match the rounded input shell.
        if !HIGH_CONTRAST && GetDlgCtrlID(h) as usize == C_ADAPTER {
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
                "选择校园网络"
            } else {
                &selected
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
        let available = scale_for_dpi(width, UI_DPI);
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
            right: scale_for_dpi(width, UI_DPI),
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
        let status_height = measured_height(h, &text(h, C_STATUS), UI_FONT, 472).clamp(44, 100);
        HERO_EXTRA = status_height - 44;
        NETWORK_TEXT_HEIGHT = measured_height(h, &text(h, C_NETWORK_VALUE), UI_FONT, 496).max(28);
        let network_y = 320 + HERO_EXTRA;
        let network_height = 96 + NETWORK_TEXT_HEIGHT;
        set_control_bounds(C_STATUS, 216, 180, 496, status_height);
        set_control_bounds(M_CHECK, 192, 256 + HERO_EXTRA, 144, 40);
        set_control_bounds(M_OPEN, 352, 256 + HERO_EXTRA, 176, 40);
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
            20,
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
        set_control_bounds(C_FOOTER, 192, footer_y, 544, 44);
        OVERVIEW_HEIGHT = CONTENT_HEIGHT.max(footer_y + 68);
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
        configure_scroll(h);
    }
    unsafe fn form_feedback(h: Handle, message: &str, field: usize) {
        FORM_ERROR_FIELD = if message.is_empty() { 0 } else { field };
        SetWindowTextW(
            GetDlgItem(h, C_FORM_FEEDBACK as i32),
            wide(message).as_ptr(),
        );
        let height = measured_height(h, message, SMALL_FONT, 544);
        FORM_SHIFT = if height == 0 { 0 } else { height + 12 };
        set_control_bounds(C_FORM_FEEDBACK, 192, 88, 544, height);
        layout_controls(h);
        configure_scroll(h);
        if field != 0 && !message.is_empty() {
            SetFocus(GetDlgItem(h, field as i32));
        }
        ensure_focus_visible(h);
    }
    unsafe fn content_height() -> i32 {
        if ACTIVE_PAGE == 2 {
            CONTENT_HEIGHT + FORM_SHIFT + FORM_NETWORK_EXTRA
        } else {
            OVERVIEW_HEIGHT
        }
    }
    unsafe fn layout_controls(h: Handle) {
        let mut client: Rect = zeroed();
        GetClientRect(h, &mut client);
        let viewport_height = ((client.bottom as i64 * 96) / UI_DPI as i64) as i32;
        let compact_sidebar = viewport_height < 328;
        let footer_visible = viewport_height >= 208;
        let footer_top = if compact_sidebar {
            (viewport_height - 56).max(160)
        } else {
            viewport_height - 120
        };
        if !footer_visible && matches!(GetDlgCtrlID(GetFocus()) as usize, C_AUTHOR | C_ABOUT) {
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
                if id == C_AUTHOR {
                    SetWindowTextW(
                        item.hwnd,
                        wide(if compact_sidebar {
                            "Liunian520 · GitHub"
                        } else {
                            "Liunian520\r\n作者 · GitHub 主页"
                        })
                        .as_ptr(),
                    );
                }
                let visible = (item.page == 0 || item.page == ACTIVE_PAGE)
                    && (!matches!(id, C_AUTHOR | C_ABOUT) || footer_visible);
                ShowWindow(item.hwnd, if visible { 5 } else { 0 });
                let offset = if item.page == 0 { 0 } else { SCROLL_OFFSET };
                let y = match id {
                    C_NAV_OVERVIEW if compact_sidebar => 88,
                    C_NAV_SETTINGS if compact_sidebar => 124,
                    C_AUTHOR if compact_sidebar => footer_top,
                    C_ABOUT if compact_sidebar => footer_top + 24,
                    C_AUTHOR => footer_top + 8,
                    C_ABOUT => footer_top + 64,
                    _ => item.y,
                };
                let height = match id {
                    C_NAV_OVERVIEW | C_NAV_SETTINGS if compact_sidebar => 32,
                    C_AUTHOR | C_ABOUT if compact_sidebar => 24,
                    _ => item.h,
                };
                let shift = if item.page == 2
                    && !matches!(id, C_SETTINGS_TITLE | C_FORM_SUBTITLE | C_FORM_FEEDBACK)
                {
                    FORM_SHIFT + if item.y >= 200 { FORM_NETWORK_EXTRA } else { 0 }
                } else {
                    0
                };
                MoveWindow(
                    item.hwnd,
                    scale_for_dpi(item.x, UI_DPI),
                    scale_for_dpi(y + shift, UI_DPI) - offset,
                    scale_for_dpi(item.w, UI_DPI),
                    scale_for_dpi(height, UI_DPI),
                    1,
                );
            }
        });
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
            let combo = GetDlgItem(h, C_ADAPTER as i32);
            let index = SendMessageW(combo, 0x147, 0, 0);
            let selected = if index >= 0 {
                app.adapters
                    .borrow()
                    .get(index as usize)
                    .map(|a| a.id.clone())
            } else {
                None
            };
            let adapters = (app.adapter_source)();
            SendMessageW(combo, 0x14b, 0, 0);
            for adapter in &adapters {
                SendMessageW(combo, 0x143, 0, wide(&adapter.name).as_ptr() as isize);
            }
            let next = selected
                .and_then(|id| adapters.iter().position(|a| a.id == id))
                .map(|n| n as isize)
                .unwrap_or(-1);
            *app.adapters.borrow_mut() = adapters;
            SendMessageW(combo, 0x14e, next as usize, 0);
            update_adapter_detail(h);
            update_overview(h);
        }
    }
    unsafe fn reset_form(h: Handle) {
        if let Some(app) = app() {
            let combo = GetDlgItem(h, C_ADAPTER as i32);
            let adapters = (app.adapter_source)();
            let settings = app.settings.borrow().clone();
            SendMessageW(combo, 0x14b, 0, 0);
            for a in &adapters {
                SendMessageW(combo, 0x143, 0, wide(&a.name).as_ptr() as isize);
            }
            let idx = adapters
                .iter()
                .position(|a| a.id == settings.adapter_id)
                .map(|i| i as isize)
                .unwrap_or(-1);
            *app.adapters.borrow_mut() = adapters;
            SendMessageW(combo, 0x14e, idx as usize, 0);
            update_adapter_detail(h);
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
            update_overview(h);
        }
    }
    unsafe fn update_adapter_detail(h: Handle) {
        let selected = SendMessageW(GetDlgItem(h, C_ADAPTER as i32), 0x147, 0, 0);
        let name = app()
            .and_then(|a| {
                a.adapters
                    .borrow()
                    .get(selected as usize)
                    .map(|value| value.name.clone())
            })
            .unwrap_or_default();
        let dc = GetDC(h);
        let overflow = if dc == 0 {
            !name.is_empty()
        } else {
            let old = SelectObject(dc, UI_FONT);
            let overflow = single_line_width(dc, &name) > scale_for_dpi(460, UI_DPI);
            SelectObject(dc, old);
            ReleaseDC(h, dc);
            overflow
        };
        let detail = if overflow {
            network_display_text(h, &name, SMALL_FONT, 504)
        } else {
            String::new()
        };
        let height = measured_height(h, &detail, SMALL_FONT, 504);
        FORM_NETWORK_EXTRA = if height == 0 { 0 } else { height + 12 };
        SetWindowTextW(
            GetDlgItem(h, C_ADAPTER_DETAIL as i32),
            wide(&detail).as_ptr(),
        );
        set_control_bounds(C_ADAPTER_DETAIL, 212, 188, 504, height);
        layout_controls(h);
        configure_scroll(h);
    }
    unsafe fn update_overview(h: Handle) {
        if let Some(app) = app() {
            let status = app
                .status
                .lock()
                .map(|value| value.clone())
                .unwrap_or_else(|_| "状态不可用".into());
            let headline = match connection_tone(&status) {
                ConnectionTone::Online => "校园网已连接",
                ConnectionTone::Attention => "连接需要关注",
                ConnectionTone::Pending => "正在确认连接",
            };
            SetWindowTextW(GetDlgItem(h, C_STATUS_HEAD as i32), wide(headline).as_ptr());
            let settings = app.settings.borrow();
            let name = app
                .adapters
                .borrow()
                .iter()
                .find(|a| a.id == settings.adapter_id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "尚未选择校园网络".into());
            let (network, detail) = name
                .rsplit_once(" / 网关 ")
                .map(|(network, gateway)| (network, format!("网关 {gateway}")))
                .unwrap_or((&name, "仅在匹配所选网络时执行认证".into()));
            SetWindowTextW(
                GetDlgItem(h, C_NETWORK_VALUE as i32),
                wide(&network_display_text(h, network, UI_FONT, 496)).as_ptr(),
            );
            SetWindowTextW(GetDlgItem(h, C_NETWORK_DESC as i32), wide(&detail).as_ptr());
            update_overview_layout(h);
            InvalidateRect(h, null(), 1);
        }
    }
    unsafe fn save_form(h: Handle) {
        if let Some(app) = app() {
            let idx = SendMessageW(GetDlgItem(h, C_ADAPTER as i32), 0x147, 0, 0);
            let id = if idx >= 0 {
                app.adapters
                    .borrow()
                    .get(idx as usize)
                    .map(|a| a.id.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let username = text(h, C_USER);
            let consent = SendMessageW(GetDlgItem(h, C_CONSENT as i32), 0xf0, 0, 0) == 1;
            let autostart = SendMessageW(GetDlgItem(h, C_START as i32), 0xf0, 0, 0) == 1;
            if !settings_allowed(&id, &username, consent) {
                let (message, field) = if id.trim().is_empty() {
                    ("请选择当前连接校园网的网卡。", C_ADAPTER)
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
                    update_overview(h);
                    show_page(h, 1);
                }
                Err(e) => report_settings_failure(h, app, &e),
            }
        }
    }
    unsafe fn menu(h: Handle) {
        if let Some(app) = app() {
            let m = CreatePopupMenu();
            let status = app
                .status
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "状态不可用".into());
            AppendMenuW(m, 3, 0, wide(&status).as_ptr());
            AppendMenuW(m, 0, M_CHECK, wide("立即检查").as_ptr());
            AppendMenuW(m, 0, M_SETTINGS, wide("设置").as_ptr());
            AppendMenuW(m, 0, M_OPEN, wide("打开校园网认证页面").as_ptr());
            AppendMenuW(m, 0, M_ABOUT, wide("关于校园网守护").as_ptr());
            AppendMenuW(m, 0, M_QUIT, wide("退出").as_ptr());
            let mut p = Point { x: 0, y: 0 };
            GetCursorPos(&mut p);
            SetForegroundWindow(h);
            TrackPopupMenu(m, 0, p.x, p.y, 0, h, null());
            DestroyMenu(m);
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
            tray.flags = 4 | if notify { 0x10 } else { 0 };
            copy_utf16(&format!("校园网守护：{status}"), &mut tray.tip);
            if notify {
                copy_utf16("校园网守护", &mut tray.info_title);
                copy_utf16(&status, &mut tray.info);
                tray.info_flags = 2;
            }
            Shell_NotifyIconW(1, &mut *tray);
        }
    }
    // The fixed-size controls remain readable at the user's DPI; on short screens
    // the form scrolls instead of scaling text down or clipping the last actions.
    unsafe fn configure_scroll(h: Handle) {
        let mut client = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(h, &mut client);
        let info = ScrollInfo {
            size: size_of::<ScrollInfo>() as u32,
            mask: 0x1 | 0x2,
            min: 0,
            max: scale_for_dpi(content_height(), UI_DPI) - 1,
            page: (client.bottom - client.top).max(1) as u32,
            position: 0,
            track_position: 0,
        };
        SetScrollInfo(h, 1, &info, 1);
        // A larger viewport can clamp the scrollbar position; keep the child
        // controls and the painted cards aligned with that new position.
        scroll_to(h, SCROLL_OFFSET);
    }
    unsafe fn scroll_to(h: Handle, position: i32) {
        let info = ScrollInfo {
            size: size_of::<ScrollInfo>() as u32,
            mask: 0x4,
            min: 0,
            max: 0,
            page: 0,
            position,
            track_position: 0,
        };
        let actual = SetScrollInfo(h, 1, &info, 1);
        if SCROLL_OFFSET != actual {
            SCROLL_OFFSET = actual;
            layout_controls(h);
        }
    }
    unsafe fn ensure_focus_visible(h: Handle) {
        let focus = GetFocus();
        if focus == 0 || GetParent(focus) != h {
            return;
        }
        if matches!(
            GetDlgCtrlID(focus) as usize,
            C_NAV_OVERVIEW | C_NAV_SETTINGS | C_AUTHOR | C_ABOUT
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
        if control.top < margin {
            scroll_to(h, SCROLL_OFFSET + control.top - margin);
        } else if control.bottom > client.bottom - margin {
            scroll_to(h, SCROLL_OFFSET + control.bottom - client.bottom + margin);
        }
    }
    unsafe fn dip_rect(x: i32, y: i32, w: i32, height: i32, scroll: bool) -> Rect {
        let offset = if scroll { SCROLL_OFFSET } else { 0 };
        Rect {
            left: scale_for_dpi(x, UI_DPI),
            top: scale_for_dpi(y, UI_DPI) - offset,
            right: scale_for_dpi(x + w, UI_DPI),
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
            right: scale_for_dpi(theme::SIDEBAR_WIDTH, UI_DPI),
            bottom: client.bottom,
        };
        FillRect(dc, &rail, SIDEBAR_BRUSH);
        DrawIconEx(
            dc,
            scale_for_dpi(20, UI_DPI),
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
            rounded(
                dc,
                &dip_rect(192, 96, 544, 144 + HERO_EXTRA, true),
                12,
                theme::HERO,
                theme::HERO,
            );
            rounded(
                dc,
                &dip_rect(192, 320 + HERO_EXTRA, 544, 96 + NETWORK_TEXT_HEIGHT, true),
                12,
                theme::SURFACE,
                theme::SURFACE_BORDER,
            );
        } else {
            for (y, height) in [(88, 96), (200, 152), (368, 152)] {
                let shift = FORM_SHIFT + if y >= 200 { FORM_NETWORK_EXTRA } else { 0 };
                rounded(
                    dc,
                    &dip_rect(
                        192,
                        y + shift,
                        544,
                        height + if y == 88 { FORM_NETWORK_EXTRA } else { 0 },
                        true,
                    ),
                    12,
                    theme::SURFACE,
                    theme::SURFACE_BORDER,
                );
            }
            for (id, x, y, width) in [
                (C_USER, 212, 272, 240),
                (C_PASS, 476, 272, 240),
                (C_ADAPTER, 212, 136, 504),
            ] {
                let shift = FORM_SHIFT + if y >= 200 { FORM_NETWORK_EXTRA } else { 0 };
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
    unsafe fn draw_button(item: &DrawItem) {
        let id = item.control_id as usize;
        let navigation = matches!(id, C_NAV_OVERVIEW | C_NAV_SETTINGS);
        let footer = matches!(id, C_AUTHOR | C_ABOUT);
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
            } else {
                BACKGROUND_BRUSH
            },
        );
        let fill = if disabled {
            theme::DISABLED_SURFACE
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
        let border = if primary || navigation || footer || tertiary {
            fill
        } else {
            theme::CONTROL_BORDER
        };
        rounded(item.dc, &item.rect, 8, fill, border);
        let color = if disabled {
            theme::DISABLED_TEXT
        } else if primary {
            theme::TEXT_ON_PRIMARY
        } else if id == C_AUTHOR {
            if pressed {
                theme::PRIMARY_PRESSED
            } else if hover {
                theme::PRIMARY_HOVER
            } else {
                theme::PRIMARY
            }
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
        let mut buf = [0u16; 128];
        let n = GetWindowTextW(item.item, buf.as_mut_ptr(), buf.len() as i32);
        let native_label = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let label = match id {
            C_NAV_OVERVIEW => "连接概览",
            C_NAV_SETTINGS => "认证设置",
            _ => &native_label,
        };
        if id == C_AUTHOR {
            let inset = scale_for_dpi(12, UI_DPI);
            let compact = bounds.bottom - bounds.top < scale_for_dpi(40, UI_DPI);
            paint_label(
                item.dc,
                "Liunian520",
                Rect {
                    left: bounds.left + inset,
                    top: bounds.top + if compact { 0 } else { scale_for_dpi(5, UI_DPI) },
                    right: bounds.right - inset,
                    bottom: if compact {
                        bounds.bottom
                    } else {
                        bounds.top + scale_for_dpi(25, UI_DPI)
                    },
                },
                UI_FONT,
                color,
                0x4 | 0x20,
            );
            if !compact {
                paint_label(
                    item.dc,
                    "作者 · GitHub 主页",
                    Rect {
                        left: bounds.left + inset,
                        top: bounds.top + scale_for_dpi(27, UI_DPI),
                        right: bounds.right - inset,
                        bottom: bounds.bottom - scale_for_dpi(3, UI_DPI),
                    },
                    SMALL_FONT,
                    if pressed {
                        theme::TEXT_SECONDARY
                    } else {
                        theme::TEXT_TERTIARY
                    },
                    0x4 | 0x20,
                );
            }
        } else {
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
                configure_scroll(h);
                layout_controls(h);
                0
            }
            WM_VSCROLL => {
                let mut info = ScrollInfo {
                    size: size_of::<ScrollInfo>() as u32,
                    mask: 0x10,
                    min: 0,
                    max: 0,
                    page: 0,
                    position: 0,
                    track_position: 0,
                };
                GetScrollInfo(h, 1, &mut info);
                let line = scale_for_dpi(40, UI_DPI);
                let next = match w & 0xffff {
                    0 => SCROLL_OFFSET - line,
                    1 => SCROLL_OFFSET + line,
                    2 => SCROLL_OFFSET - line * 4,
                    3 => SCROLL_OFFSET + line * 4,
                    4 | 5 => info.track_position,
                    6 => 0,
                    7 => scale_for_dpi(content_height(), UI_DPI),
                    _ => SCROLL_OFFSET,
                };
                scroll_to(h, next);
                0
            }
            WM_MOUSEWHEEL => {
                let delta = ((w >> 16) as u16 as i16) as i32;
                scroll_to(h, SCROLL_OFFSET - delta * scale_for_dpi(120, UI_DPI) / 120);
                0
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
                        | C_ABOUT
                        | M_CHECK
                        | M_OPEN
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
                ShowWindow(h, 0);
                0
            }
            WM_DESTROY => {
                KillTimer(h, 1);
                if let Some(app) = app() {
                    Shell_NotifyIconW(2, &mut *app.tray.borrow_mut());
                    notify_quit(&app.generation, &app.gate, &app.tx);
                }
                PostQuitMessage(0);
                0
            }
            WM_TIMER => {
                if w == 1 {
                    refresh_status();
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
            TRAY => {
                if l as u32 == WM_RBUTTONUP {
                    menu(h)
                } else if l as u32 == WM_LBUTTONDBLCLK {
                    open_panel(h, 1);
                }
                0
            }
            WM_COMMAND => {
                let id = w & 0xffff;
                let notification = w >> 16;
                if id == C_ADAPTER && notification == 1 {
                    update_adapter_detail(h);
                }
                if FORM_ERROR_FIELD != 0
                    && id == FORM_ERROR_FIELD
                    && ((matches!(id, C_USER | C_PASS) && notification == 0x300)
                        || (id == C_ADAPTER && notification == 1)
                        || (id == C_CONSENT && notification == 0))
                {
                    form_feedback(h, "", 0);
                }
                match id {
                    C_NAV_OVERVIEW => show_page(h, 1),
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
                    C_ABOUT | M_ABOUT => show_about(h),
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
                    M_SETTINGS => {
                        open_panel(h, 2);
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
            let work_height =
                if SystemParametersInfoW(0x30, 0, (&mut work_area as *mut Rect).cast(), 0) != 0 {
                    (work_area.bottom - work_area.top).max(200)
                } else {
                    scale_for_dpi(700, UI_DPI)
                };
            let window_style = 0x00ca0000 | 0x00200000;
            let mut frame = Rect {
                left: 0,
                top: 0,
                right: scale_for_dpi(760, UI_DPI),
                bottom: scale_for_dpi(CONTENT_HEIGHT, UI_DPI),
            };
            AdjustWindowRectEx(&mut frame, window_style, 0, 0);
            // AdjustWindowRectEx excludes the vertical scrollbar's width.
            frame.right += GetSystemMetrics(2);
            let h = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("校园网守护").as_ptr(),
                window_style,
                0x80000000u32 as i32,
                0x80000000u32 as i32,
                frame.right - frame.left,
                (frame.bottom - frame.top).min(work_height),
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
                488,
                144,
                48,
                C_AUTHOR,
            );
            control(
                h,
                "BUTTON",
                &format!("关于 · v{} · MIT", application_version()),
                WS_TABSTOP | button_style,
                12,
                544,
                144,
                32,
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
                "正常时每 60 秒检查，网络变化后重新确认。\r\n关闭窗口后继续在系统托盘运行。",
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
                "无线请先连接校园 Wi-Fi；\r\n列表缺失时检查 Windows 位置权限。",
                0,
                308,
                96,
                408,
                40,
                C_HINT_NETWORK,
            );
            control(
                h,
                "COMBOBOX",
                "",
                WS_TABSTOP | 0x3 | 0x200000,
                222,
                142,
                484,
                210,
                C_ADAPTER,
            );
            SendMessageW(
                GetDlgItem(h, C_ADAPTER as i32),
                0x153,
                usize::MAX,
                scale_for_dpi(23, UI_DPI) as isize,
            );
            control(h, "STATIC", "", 0, 212, 188, 504, 0, C_ADAPTER_DETAIL);
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
                "密码由 Windows 凭据管理器保存；留空保留原密码。",
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
                "我了解并同意使用 HTTP 自动认证",
                WS_TABSTOP | 3,
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
                WS_TABSTOP | 3,
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
                "保存并开始守护",
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
                "删除保存的密码",
                WS_TABSTOP | button_style,
                560,
                536,
                176,
                40,
                C_DELETE,
            );
            BUILD_PAGE = 0;
            let first = initial.adapter_id.is_empty()
                || initial.username.is_empty()
                || !initial.http_acknowledged;
            let mut tray: Notify = zeroed();
            tray.size = size_of::<Notify>() as u32;
            tray.hwnd = h;
            tray.id = 1;
            tray.flags = 1 | 2 | 4;
            tray.callback = TRAY;
            tray.icon = icon;
            for (to, from) in tray.tip.iter_mut().zip(wide("校园网守护")) {
                *to = from;
            }
            let state = Box::new(App {
                settings: RefCell::new(initial),
                adapters: RefCell::new(Vec::new()),
                adapter_source,
                tx,
                status,
                tray: RefCell::new(tray),
                last_status: RefCell::new(String::new()),
                generation,
                gate,
            });
            APP = &*state;
            if Shell_NotifyIconW(0, &mut *state.tray.borrow_mut()) == 0 {
                APP = null();
                DestroyWindow(h);
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
                return Err(error("创建托盘图标失败"));
            }
            reset_form(h);
            show_page(h, if first { 2 } else { 1 });
            SetTimer(h, 1, 1000, null());
            refresh_status();
            if first || std::env::args().any(|arg| arg == "--show") {
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
                if IsDialogMessageW(h, &mut msg) == 0 {
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
