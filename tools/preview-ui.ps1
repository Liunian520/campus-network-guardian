<#
.SYNOPSIS
Build and capture an isolated native UI with fake data, without running the worker.
.DESCRIPTION
The preview lives off screen and does not receive foreground focus. Validation
checks may click Save only with deliberately invalid fake fields that return
before any credential, registry, or controller transaction. It never clicks
Delete or the portal link. Diagnostic buttons use fake data and do not perform
network operations or write to the clipboard. Captures use PrintWindow rather
than the desktop so other applications and real account data are excluded.
The matching preview PID is always closed, including on screenshot failure.
#>
param(
    [ValidateSet('overview', 'settings', 'all')]
    [string]$Page = 'all',
    [ValidateSet(96, 120, 144, 192)]
    [int]$Dpi = 96,
    [ValidateSet('online', 'waiting', 'wifi-mac', 'long-error', 'very-long', 'dual')]
    [string]$Status = 'online',
    [string]$ResourceLibrary,
    [ValidateRange(0, 8192)]
    [int]$WorkAreaWidth = 0,
    [int]$WindowHeight = 0,
    [switch]$ScrollBottom,
    [switch]$ClientOnly,
    [switch]$VerifyNavigation,
    [switch]$VerifyAuthor,
    [switch]$VerifyUpdateCard,
    [switch]$VerifyUpdateNotice,
    [switch]$VerifyUpdateProgress,
    [switch]$VerifyPageScrollbar,
    [switch]$AlwaysShowPageScrollbar,
    [switch]$VerifyTrayActivation,
    [switch]$VerifyValidation,
    [switch]$VerifyAnalytics,
    [switch]$VerifyDiagnostics,
    [switch]$LongAdapter,
    [switch]$UnbrokenAdapter,
    [switch]$DiagnoseStatus
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildDir = Join-Path $workspace 'target\ui-review'
New-Item -ItemType Directory -Path $buildDir -Force | Out-Null
$captureDir = Join-Path ([IO.Path]::GetTempPath()) ('campus-guardian-ui-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '-' + $PID)
New-Item -ItemType Directory -Path $captureDir -Force | Out-Null

if (-not $ResourceLibrary) {
    $ResourceLibrary = Get-ChildItem -LiteralPath (Join-Path $workspace 'target') -Filter 'guardian.lib' -Recurse |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $ResourceLibrary -or -not (Test-Path -LiteralPath $ResourceLibrary -PathType Leaf)) {
    throw 'No compiled guardian.lib found. Build the application once before using this preview.'
}

$previewBase = Join-Path $buildDir ('preview-' + (Get-Date -Format 'yyyyMMddHHmmssfff') + '-' + $PID)
$previewSource = $previewBase + '.rs'
$previewExe = $previewBase + '.exe'
$uiSource = (Join-Path $workspace 'src\windows_ui.rs').Replace('\', '/')
$previewText = @'
#![windows_subsystem = "windows"]
#![allow(dead_code, unused_imports)]
#[path = "__UI_SOURCE__"] mod windows_ui;
use std::sync::{atomic::AtomicU64, mpsc, Arc, Mutex};
use std::{cell::Cell, rc::Rc, time::{Duration, Instant}};
#[link(name = "user32")]
extern "system" {
    fn EnumWindows(callback: unsafe extern "system" fn(isize, isize) -> i32, parameter: isize) -> i32;
    fn GetWindowThreadProcessId(window: isize, process_id: *mut u32) -> u32;
    fn PostMessageW(window: isize, message: u32, w: usize, l: isize) -> i32;
}
unsafe extern "system" fn close_preview_window(window: isize, expected_pid: isize) -> i32 {
    let mut actual_pid = 0;
    GetWindowThreadProcessId(window, &mut actual_pid);
    if actual_pid == expected_pid as u32 { PostMessageW(window, 0x111, 204, 0); }
    1
}
fn update_fixture() -> Option<(&'static str, &'static str, Option<(u64, u64)>)> {
    const MIB: u64 = 1024 * 1024;
    let path = std::env::var_os("CAMPUS_PREVIEW_UPDATE_FIXTURE")?;
    let stage = std::fs::read_to_string(path).ok()?;
    let fixture = match stage.trim() {
        "select" => ("正在测试下载线路", "流年 App · CF CDN", None),
        "start" => ("正在下载安装包", "流年 App · CF CDN · 多线程", Some((8 * MIB, 32 * MIB))),
        "advance" => ("正在下载安装包", "流年 App · CF CDN · 多线程", Some((20 * MIB, 32 * MIB))),
        "retry" => ("当前线路失败，正在切换", "流年 App · CF CDN · 多线程", None),
        "restart" => ("已切换线路，正在重新下载", "GitHub 镜像 · 单线程", Some((4 * MIB, 32 * MIB))),
        "verify" => ("正在校验安装包", "GitHub 镜像 · 单线程", None),
        _ => return None,
    };
    Some(fixture)
}
fn main() {
    // Deliberately omit main.rs, the network worker, configuration loading,
    // single-instance handling, credential reads, and registry reads.
    // Also retire automatically if the controlling PowerShell host terminates.
    // WM_COMMAND/204 follows the application's normal tray-cleanup path.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(30));
        unsafe { EnumWindows(close_preview_window, std::process::id() as isize); }
    });
    let (tx, _rx) = mpsc::channel();
    let selected_status = std::env::var("CAMPUS_PREVIEW_STATUS");
    let status: String = match selected_status.as_deref() {
        Ok("waiting") => "网络已变化，正在等待网络稳定".into(),
        Ok("wifi-mac") => "校园 Wi-Fi 随机 MAC 已关闭；等待下次连接后确认固定地址。自动认证暂缓，不主动断开当前网络。".into(),
        // Keep this distinct from real notification strings so the isolated
        // preview cannot show a system notification to the user.
        Ok("long-error") => "门户提示账号异常，请打开认证页面检查；演示状态：这是一条较长的恢复提示，用于检查换行和内容布局。".into(),
        Ok("very-long") => "门户提示账号异常，请打开认证页面检查；演示状态：这是一条较长的恢复提示，用于检查换行和内容布局。".repeat(12),
        Ok("dual") => "Wi-Fi：校园网在线\r\n以太网：校园网络不匹配或不可用，已暂停\r\n系统选路：WHUT-ISP 可用，等待以太网".into(),
        _ => "校园网在线".into(),
    };
    let adapter_name = if std::env::var("CAMPUS_PREVIEW_UNBROKEN_ADAPTER").as_deref() == Ok("1") {
        // One ASCII run: DT_WORDBREAK can expand the measured width without
        // increasing height, so this cannot rely on ordinary word wrapping.
        format!("preview-adapter-{}-WHUT-ISP", "W".repeat(200))
    } else if std::env::var("CAMPUS_PREVIEW_LONG_ADAPTER").as_deref() == Ok("1") {
        "预览网卡 · WHUT-ISP · USB 以太网适配器（用于验证较长中文与英文名称的动态换行、网关独立显示以及高 DPI 布局） / 网关 10.0.0.1".to_string()
    } else {
        "预览网卡 · WHUT-ISP / 网关 10.0.0.1".to_string()
    };
    let requested = Rc::new(Cell::new(None::<Instant>));
    let request_time = Rc::clone(&requested);
    let info_time = Rc::clone(&requested);
    let analytics_enabled = Rc::new(Cell::new(false));
    let analytics_preference = Rc::clone(&analytics_enabled);
    let external_requested = Rc::new(Cell::new(None::<Instant>));
    let external_result = Rc::clone(&external_requested);
    let dual = selected_status.as_deref() == Ok("dual");
    let diagnostics_summary = move || {
        let external = match external_result.get() {
            None => "未检测",
            Some(start) if start.elapsed() < Duration::from_millis(350) => "正在检测",
            Some(_) => "可访问",
        };
        let wifi = format!("Wi-Fi：网络接入 已匹配 · 校园网认证 已认证\r\n运营商拨号 无需拨号 · IPv4 外网 {external} · 下次检查 约 45 秒后");
        if dual {
            format!("{wifi}\r\n\r\n以太网：网络接入 未连接或不可用 · 校园网认证 待检查\r\n运营商拨号 待确认 · IPv4 外网 未检测")
        } else {
            wifi
        }
    };
    let report_summary = diagnostics_summary.clone();
    let result = windows_ui::run(
        windows_ui::Settings {
            adapter_id: "preview-adapter".into(),
            secondary_adapter_id: if selected_status.as_deref() == Ok("dual") { "preview-alternative".into() } else { String::new() },
            username: "demo-campus".into(),
            http_acknowledged: true,
            autostart: false,
        },
        Box::new(move || vec![windows_ui::Adapter {
            id: "preview-adapter".into(),
            name: adapter_name.clone(),
        }, windows_ui::Adapter {
            id: "preview-alternative".into(),
            name: "预览备用网卡 / 网关 10.0.0.2".into(),
        }]),
        tx, Arc::new(Mutex::new(status)),
        Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(())),
        windows_ui::UiUpdates {
            request: Box::new(move || requested.set(Some(Instant::now()))),
            status: Box::new(move || {
                if update_fixture().is_some() {
                    return windows_ui::UpdateState::Checking;
                }
                match request_time.get() {
                    None => windows_ui::UpdateState::Idle,
                    Some(start) if start.elapsed() < Duration::from_millis(1200) => windows_ui::UpdateState::Checking,
                    Some(_) => windows_ui::UpdateState::Current,
                }
            }),
            info: Box::new(move || {
                let fallback_phase = match info_time.get() {
                    None => "尚未开始检查",
                    Some(start) if start.elapsed() < Duration::from_millis(1200) => "正在检查更新",
                    Some(_) => "更新检查已完成",
                };
                let fixture = update_fixture();
                let (progress, source, transfer) = fixture.unwrap_or((fallback_phase, "流年 App · CF CDN", None));
                windows_ui::UpdateInfo {
                    version: if fixture.is_some() { "0.7.2" } else { "0.6.8" }.into(),
                    source: source.into(),
                    changelog: format!("  ## 本版更新\n\t- 自动选择更新线路\n    * 支持分段下载与完整校验\n\n  安装与使用\n1. 下载完整安装包\n2. 按提示完成更新\n\n  作者：Liunian520\n  https://github.com/Liunian520/campus-network-guardian\n\n{}", "  更新说明：这是隔离预览中的演示公告，可滚动并复制。\n".repeat(18)),
                    progress: progress.into(),
                    transfer,
                }
            }),
        },
        windows_ui::UiAnalytics {
            enabled: Box::new(move || analytics_preference.get()),
            set_enabled: Box::new(move |enabled| {
                analytics_enabled.set(enabled);
                Ok(())
            }),
        },
        windows_ui::UiDiagnostics {
            summary: Box::new(diagnostics_summary),
            report: Box::new(move || format!("校园网守护诊断报告\r\n{}\r\n\r\n最近恢复记录\r\n{}",
                report_summary(), "  刚刚 · Wi-Fi · 校园网认证：已认证（演示）\r\n".repeat(30))),
            check_external: Box::new(move || external_requested.set(Some(Instant::now()))),
        },
    );
    if let Err(error) = result {
        std::fs::write(std::env::var_os("CAMPUS_PREVIEW_ERROR").unwrap_or_default(), error).ok();
        std::process::exit(1);
    }
}
'@
[IO.File]::WriteAllText($previewSource, $previewText.Replace('__UI_SOURCE__', $uiSource), [Text.UTF8Encoding]::new($false))
$manifest = Get-Content -LiteralPath (Join-Path $workspace 'Cargo.toml') -Raw
$versionMatch = [regex]::Match($manifest, '(?ms)^\[package\]\s*.*?^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"')
if (-not $versionMatch.Success) { throw 'The package version is missing from Cargo.toml.' }
$priorPackageVersion = [Environment]::GetEnvironmentVariable('CARGO_PKG_VERSION')
try {
    $env:CARGO_PKG_VERSION = $versionMatch.Groups[1].Value
    & rustc --edition 2021 --crate-name guardian_ui_preview $previewSource -C opt-level=1 -C debug-assertions=yes -C "link-arg=$ResourceLibrary" -o $previewExe
    if ($LASTEXITCODE -ne 0) { throw 'Preview compilation failed.' }
} finally {
    [Environment]::SetEnvironmentVariable('CARGO_PKG_VERSION', $priorPackageVersion)
}

Add-Type -AssemblyName System.Drawing
if (-not ('GuardianPreviewNative' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class GuardianPreviewNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct ScrollInfo {
        public uint Size, Mask; public int Min, Max; public uint Page; public int Position, TrackPosition;
    }
    [StructLayout(LayoutKind.Sequential)] public struct GuiThreadInfo {
        public uint Size, Flags; public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret; public Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct NotifyIconData {
        public uint Size; public IntPtr Window; public uint Id, Flags, Callback; public IntPtr Icon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string Tip;
        public uint State, StateMask;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=256)] public string Info;
        public uint Timeout;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=64)] public string InfoTitle;
        public uint InfoFlags; public Guid Guid; public IntPtr BalloonIcon;
    }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder name, int count);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern int MapWindowPoints(IntPtr from, IntPtr to, ref Rect points, uint count);
    [DllImport("user32.dll")] public static extern bool GetScrollInfo(IntPtr window, int bar, ref ScrollInfo info);
    [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetWindowStyle(IntPtr window, int index);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool EnableWindow(IntPtr window, bool enabled);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint threadId, ref GuiThreadInfo info);
    // Cross-process SetWindowText/GetWindowText only address cached captions.
    // EDIT contents must use the marshalled WM_SETTEXT/WM_GETTEXT messages.
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] private static extern IntPtr SendMessageString(IntPtr window, uint message, IntPtr w, string text);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] private static extern IntPtr SendMessageBuffer(IntPtr window, uint message, IntPtr w, StringBuilder text);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessageRect(IntPtr window, uint message, IntPtr w, ref Rect rect);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr window, IntPtr rect, IntPtr region, uint flags);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern bool Shell_NotifyIcon(uint message, ref NotifyIconData data);
    public static bool SetControlText(IntPtr window, string text) {
        return SendMessageString(window, 0xc, IntPtr.Zero, text).ToInt64() != 0;
    }
    public static string ReadControlText(IntPtr window) {
        var text = new StringBuilder(4096);
        SendMessageBuffer(window, 0xd, new IntPtr(text.Capacity), text);
        return text.ToString();
    }
    public static IntPtr FindPreviewWindow(uint expectedPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            uint actualPid; GetWindowThreadProcessId(window, out actualPid);
            if (actualPid != expectedPid) return true;
            var name = new StringBuilder(256); GetClassName(window, name, name.Capacity);
            if (!name.ToString().Contains("Guardian")) return true;
            found = window; return false;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr FindPreviewNotice(uint expectedPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            uint actualPid; GetWindowThreadProcessId(window, out actualPid);
            if (actualPid != expectedPid) return true;
            var name = new StringBuilder(256); GetClassName(window, name, name.Capacity);
            if (name.ToString() != "CampusGuardianUpdateNotice") return true;
            found = window; return false;
        }, IntPtr.Zero);
        return found;
    }
    public static void RemovePreviewTray(IntPtr window) {
        var data = new NotifyIconData { Window=window, Id=1, Tip="", Info="", InfoTitle="" };
        data.Size=(uint)Marshal.SizeOf(typeof(NotifyIconData));
        Shell_NotifyIcon(2, ref data);
    }
    public static IntPtr PreviewFocus(IntPtr window) {
        uint pid; uint thread = GetWindowThreadProcessId(window, out pid);
        var info = new GuiThreadInfo(); info.Size=(uint)Marshal.SizeOf(typeof(GuiThreadInfo));
        return GetGUIThreadInfo(thread, ref info) ? info.Focus : IntPtr.Zero;
    }
}
'@
}

function Test-PreviewNavigation([IntPtr]$Window) {
    $report = [Collections.Generic.List[string]]::new()
    foreach ($navigationPage in @('overview', 'settings')) {
        $navId = if ($navigationPage -eq 'overview') { 301 } else { 302 }
        $nav = [GuardianPreviewNative]::GetDlgItem($Window, $navId)
        [GuardianPreviewNative]::SendMessage($nav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        [GuardianPreviewNative]::PostMessage($Window, 0x28, $nav, [IntPtr]1) | Out-Null
        Start-Sleep -Milliseconds 100
        $ids = [Collections.Generic.List[int]]::new()
        $allowed = if ($navigationPage -eq 'overview') { @(301,302,303,304,305,306,110,201,203,207,142,143) } else { @(301,302,303,304,305,306,101,135,102,103,104,105,106,107,138) }
        for ($tab = 0; $tab -lt 20; $tab++) {
            $focus = [GuardianPreviewNative]::PreviewFocus($Window)
            if ($focus -eq [IntPtr]::Zero) { throw 'Preview does not expose background keyboard focus; no foreground activation was attempted.' }
            $id = [GuardianPreviewNative]::GetDlgCtrlID($focus)
            if (-not [GuardianPreviewNative]::IsWindowVisible($focus) -or $id -notin $allowed) {
                throw "Tab reached hidden or unexpected control $id on $navigationPage."
            }
            $ids.Add($id)
            # Only TAB is posted; Space and Enter are intentionally never used.
            [GuardianPreviewNative]::PostMessage($focus, 0x100, [IntPtr]9, [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 30
        }
        $missing = @($allowed | Where-Object { $_ -notin $ids })
        if ($missing.Count -gt 0) {
            throw "Tab did not reach all controls on $navigationPage; missing: $($missing -join ', '). Observed: $($ids -join ', ')."
        }
        $report.Add("$navigationPage Tab focus IDs: $($ids -join ', ')")
    }
    $settingsNav = [GuardianPreviewNative]::GetDlgItem($Window, 302)
    $overviewNav = [GuardianPreviewNative]::GetDlgItem($Window, 301)
    [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $secondary = [GuardianPreviewNative]::GetDlgItem($Window, 135)
    $username = [GuardianPreviewNative]::GetDlgItem($Window, 102)
    $password = [GuardianPreviewNative]::GetDlgItem($Window, 103)
    [GuardianPreviewNative]::SetControlText($username, 'demo-edited') | Out-Null
    [GuardianPreviewNative]::SetControlText($password, 'fake-preview-draft-only') | Out-Null
    [GuardianPreviewNative]::SendMessage($secondary, 0x14e, [IntPtr]2, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SendMessage($overviewNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ([GuardianPreviewNative]::ReadControlText($username) -ne 'demo-edited') { throw 'Navigating between pages discarded the fake username edit.' }
    # Native password EDIT intentionally denies cross-process WM_GETTEXT;
    # WM_GETTEXTLENGTH verifies the nonempty fake draft without reading it.
    $draftLength = [GuardianPreviewNative]::SendMessage($password, 0xe, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    if ($draftLength -ne 'fake-preview-draft-only'.Length) { throw 'Navigating between pages discarded the fake password draft.' }
    if ([GuardianPreviewNative]::SendMessage($secondary, 0x147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 2) { throw 'Navigating between pages discarded the fake second network.' }
    [GuardianPreviewNative]::SendMessage($secondary, 0x14e, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SetControlText($username, 'demo-campus') | Out-Null
    [GuardianPreviewNative]::SetControlText($password, '') | Out-Null
    $report.Add('Unsaved fake username, password, and second network survive settings -> overview -> settings.')
    $combo = [GuardianPreviewNative]::GetDlgItem($Window, 101)
    [GuardianPreviewNative]::SendMessage($combo, 0x14e, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SendMessage($combo, 0x100, [IntPtr]40, [IntPtr]::Zero) | Out-Null
    $down = [GuardianPreviewNative]::SendMessage($combo, 0x147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    [GuardianPreviewNative]::SendMessage($combo, 0x100, [IntPtr]38, [IntPtr]::Zero) | Out-Null
    $up = [GuardianPreviewNative]::SendMessage($combo, 0x147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    if ($down -ne 1 -or $up -ne 0) { throw "Closed combo keyboard selection failed: down=$down, up=$up." }
    $report.Add('Closed native combo Down/Up selected and restored the fake adapter.')
    if ($Status -eq 'very-long') {
        [GuardianPreviewNative]::SendMessage($overviewNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        $statusEdit = [GuardianPreviewNative]::GetDlgItem($Window, 110)
        $lineCount = [GuardianPreviewNative]::SendMessage($statusEdit, 0xba, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        [GuardianPreviewNative]::SendMessage($statusEdit, 0xb6, [IntPtr]::Zero, [IntPtr]20) | Out-Null
        $firstLine = [GuardianPreviewNative]::SendMessage($statusEdit, 0xce, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        if ($lineCount -lt 3 -or $firstLine -le 0) { throw "Long status could not scroll: lines=$lineCount, firstVisibleLine=$firstLine." }
        $report.Add("Long read-only status scrolls: $lineCount lines; first visible line after scrolling=$firstLine.")
    }
    if ($WindowHeight -gt 0) {
        [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        # This is a native top-level window, not a DefDlgProc dialog;
        # WM_NEXTDLGCTL does not move focus. Traverse the actual Tab path.
        for ($step = 0; $step -lt 12; $step++) {
            $focus = [GuardianPreviewNative]::PreviewFocus($Window)
            if ([GuardianPreviewNative]::GetDlgCtrlID($focus) -eq 102) { break }
            [GuardianPreviewNative]::PostMessage($focus, 0x100, [IntPtr]9, [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 30
        }
        if ([GuardianPreviewNative]::GetDlgCtrlID([GuardianPreviewNative]::PreviewFocus($Window)) -ne 102) { throw 'Could not focus the fake username via native Tab traversal.' }
        $before = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($username, [ref]$before) | Out-Null
        $scroll = [GuardianPreviewNative+ScrollInfo]::new()
        $scroll.Size = [Runtime.InteropServices.Marshal]::SizeOf($scroll)
        $scroll.Mask = 7
        $bar = [GuardianPreviewNative]::GetDlgItem($Window, 306)
        if (-not [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll)) { throw 'Could not read the isolated short-screen scroll range.' }
        $expectedBottom = [Math]::Max($scroll.Min, $scroll.Max - [int]$scroll.Page + 1)
        # Manual scrolling must remain possible while a form field has focus.
        # This does not move the pointer, type, or activate the real desktop.
        [GuardianPreviewNative]::SendMessage($Window, 0x115, [IntPtr]7, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 150
        $after = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($username, [ref]$after) | Out-Null
        [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll) | Out-Null
        $focusId = [GuardianPreviewNative]::GetDlgCtrlID([GuardianPreviewNative]::PreviewFocus($Window))
        if ($focusId -ne 102 -or $scroll.Position -ne $expectedBottom) {
            throw "Manual short-screen scroll snapped back to the focused field: focus=$focusId, scrollPosition=$($scroll.Position), requestedBottom=$expectedBottom, fieldTopBefore=$($before.Top), fieldTopAfter=$($after.Top)."
        }
        $report.Add('Short-screen manual scrolling remains at the requested position while the username field retains focus.')
    }
    return $report
}

function Measure-PreviewScrollThumb([IntPtr]$Window, [IntPtr]$Bar, [string]$State) {
    $parent = [GuardianPreviewNative+Rect]::new()
    if ($ClientOnly) {
        [GuardianPreviewNative]::GetClientRect($Window, [ref]$parent) | Out-Null
        [GuardianPreviewNative]::MapWindowPoints($Window, [IntPtr]::Zero, [ref]$parent, 2) | Out-Null
    } else {
        [GuardianPreviewNative]::GetWindowRect($Window, [ref]$parent) | Out-Null
    }
    $rail = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetWindowRect($Bar, [ref]$rail) | Out-Null
    $capture = Join-Path $captureDir "settings-scrollbar-$State.png"
    Save-PreviewWindow $Window $capture
    $bitmap = [Drawing.Bitmap]::new($capture)
    try {
        $center = [int][Math]::Floor(($rail.Left + $rail.Right) / 2) - $parent.Left
        $paintedPixels = 0
        for ($y = $rail.Top - $parent.Top; $y -lt $rail.Bottom - $parent.Top; $y++) {
            $pixel = $bitmap.GetPixel($center, $y)
            if (($pixel.ToArgb() -band 0xffffff) -in 0x707780, 0x525e6b, 0x3d4956) { $paintedPixels++ }
        }
        return $paintedPixels
    } finally {
        $bitmap.Dispose()
    }
}

function Test-PreviewPageScrollbar([IntPtr]$Window) {
    $bar = [GuardianPreviewNative]::GetDlgItem($Window, 306)
    $class = [Text.StringBuilder]::new(32)
    [GuardianPreviewNative]::GetClassName($bar, $class, $class.Capacity) | Out-Null
    if ($class.ToString() -ine 'ScrollBar' -or
        ([GuardianPreviewNative]::GetWindowStyle($Window, -16).ToInt64() -band 0x200000) -ne 0) {
        throw 'The form still uses a scrollbar on the outer window frame.'
    }
    $overview = [GuardianPreviewNative]::GetDlgItem($Window, 301)
    $settings = [GuardianPreviewNative]::GetDlgItem($Window, 302)
    [GuardianPreviewNative]::SendMessage($settings, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $scroll = [GuardianPreviewNative+ScrollInfo]::new()
    $scroll.Size = [Runtime.InteropServices.Marshal]::SizeOf($scroll)
    $scroll.Mask = 7
    if (-not [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll) -or
        -not [GuardianPreviewNative]::IsWindowVisible($bar) -or $scroll.Max -lt $scroll.Page) {
        throw 'A short settings page must expose the native scrollbar and a usable range.'
    }
    $restingPixels = Measure-PreviewScrollThumb $Window $bar 'idle'
    if ($AlwaysShowPageScrollbar) {
        if ($restingPixels -lt 10) { throw 'Always-show mode must retain a visible resting thumb.' }
    } elseif ($restingPixels -ne 0) {
        throw 'The scrollbar is still drawn while the overflowing settings page is idle.'
    }
    $bottom = $scroll.Max - [int]$scroll.Page + 1
    [GuardianPreviewNative]::SendMessage($Window, 0x20a, [IntPtr](65416L -shl 16), [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll) | Out-Null
    if ($scroll.Position -le 0) { throw 'A downward mouse wheel event did not move the page scrollbar.' }
    if ((Measure-PreviewScrollThumb $Window $bar 'scrolling') -lt 10) {
        throw 'Wheel movement did not reveal the scrollbar thumb.'
    }
    if (-not $AlwaysShowPageScrollbar) {
        Start-Sleep -Milliseconds 1200
        if ((Measure-PreviewScrollThumb $Window $bar 'settled') -ne 0) {
            throw 'The scrollbar did not clear after the wheel stopped.'
        }
    }
    [GuardianPreviewNative]::SendMessage($bar, 0x100, [IntPtr]35, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll) | Out-Null
    if ($scroll.Position -ne $bottom) { throw 'End on the native scrollbar did not reveal the end of the page.' }
    [GuardianPreviewNative]::SendMessage($bar, 0x101, [IntPtr]35, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SendMessage($bar, 0x100, [IntPtr]36, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll) | Out-Null
    if ($scroll.Position -ne 0) { throw 'Home on the native scrollbar did not return to the top of the page.' }
    [GuardianPreviewNative]::SendMessage($bar, 0x101, [IntPtr]36, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SendMessage($overview, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::GetScrollInfo($bar, 2, [ref]$scroll) | Out-Null
    if (($scroll.Max -lt $scroll.Page) -and [GuardianPreviewNative]::IsWindowVisible($bar)) {
        throw 'The page scrollbar is still visible even though all overview content fits.'
    }
    'Native scrollbar respects its resting visibility, reveals on wheel, and preserves Home/End and overflow-only interaction.'
}

function Test-PreviewAuthor([IntPtr]$Window) {
    # Inspect native names, Tab style, and geometry only. Never activate the
    # external link or show a modal dialog on the person's desktop.
    $windowRect = [GuardianPreviewNative+Rect]::new()
    $navigationRect = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetClientRect($Window, [ref]$windowRect) | Out-Null
    [GuardianPreviewNative]::MapWindowPoints($Window, [IntPtr]::Zero, [ref]$windowRect, 2) | Out-Null
    [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($Window, 302), [ref]$navigationRect) | Out-Null
    $priorBottom = $navigationRect.Bottom
    $entries = [Collections.Generic.List[GuardianPreviewNative+Rect]]::new()
    foreach ($entry in @(
        @{ Id=303; Label='Liunian520'; Role='GitHub' },
        @{ Id=305; Label='检查更新'; Role="v$($versionMatch.Groups[1].Value)" },
        @{ Id=304; Label='关于'; Role='MIT' }
    )) {
        $control = [GuardianPreviewNative]::GetDlgItem($Window, $entry.Id)
        if ($control -eq [IntPtr]::Zero) { throw "Author entry $($entry.Id) is missing." }
        $label = [GuardianPreviewNative]::ReadControlText($control)
        $style = [GuardianPreviewNative]::GetWindowStyle($control, -16).ToInt64()
        if (-not [GuardianPreviewNative]::IsWindowVisible($control) -or ($style -band 0x10000) -eq 0 -or
            -not $label.Contains($entry.Label) -or -not $label.Contains($entry.Role)) {
            throw "Author entry $($entry.Id) is not a visible, keyboard-focusable, clearly named button: '$label'."
        }
        $rect = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($control, [ref]$rect) | Out-Null
        if ($rect.Top -lt $priorBottom -or $rect.Top -lt $windowRect.Top -or
            $rect.Bottom -gt $windowRect.Bottom -or $rect.Left -lt $windowRect.Left -or
            $rect.Right -gt $windowRect.Right -or $rect.Right -le $rect.Left) {
            throw "Author entry $($entry.Id) overlaps navigation or leaves the available window."
        }
        foreach ($other in $entries) {
            if ($rect.Left -lt $other.Right -and $other.Left -lt $rect.Right -and
                $rect.Top -lt $other.Bottom -and $other.Top -lt $rect.Bottom) {
                throw "Sidebar footer entry $($entry.Id) overlaps another footer entry."
            }
        }
        $entries.Add($rect)
    }
    'Author, update and About entries have clear native names, Tab support and unobscured sidebar bounds.'
}

function Test-PreviewUpdateCard([IntPtr]$Window) {
    $control = [GuardianPreviewNative]::GetDlgItem($Window, 305)
    if ($control -eq [IntPtr]::Zero -or -not [GuardianPreviewNative]::IsWindowVisible($control)) {
        throw 'The update card is missing from the isolated preview.'
    }
    [GuardianPreviewNative]::SendMessage($control, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $checking = $false
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    while ($elapsed.ElapsedMilliseconds -lt 3500) {
        $caption = [GuardianPreviewNative]::ReadControlText($control)
        if ($caption.Contains('正在检查')) { $checking = $true }
        if ($caption.Contains('已是最新版本')) {
            if (-not $checking) { throw 'The update card skipped feedback while the check was running.' }
            return 'Update card invokes the preview check and exposes checking and current status through its native button name.'
        }
        Start-Sleep -Milliseconds 50
    }
    throw "The isolated update card did not display the completed check. Last caption: $caption"
}

function Test-PreviewUpdateNotice([IntPtr]$Window) {
    $control = [GuardianPreviewNative]::GetDlgItem($Window, 305)
    [GuardianPreviewNative]::SendMessage($control, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $notice = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        $notice = [GuardianPreviewNative]::FindPreviewNotice($previewProcess.Id)
        if ($notice -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 50
    }
    if ($notice -eq [IntPtr]::Zero) { throw 'The isolated update action did not open its native announcement window.' }
    $title = [GuardianPreviewNative]::ReadControlText($notice)
    if ($title -ne '更新与公告') { throw "The announcement window lacks its native title: '$title'." }
    $bounds = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetWindowRect($notice, [ref]$bounds) | Out-Null
    if ($bounds.Right -gt -10000 -or $bounds.Bottom -gt -10000) { throw 'The isolated notice escaped its off-screen preview owner.' }
    $noticeWidth = $bounds.Right - $bounds.Left
    $noticeHeight = $bounds.Bottom - $bounds.Top
    if (($WorkAreaWidth -gt 0 -and $noticeWidth -gt $WorkAreaWidth) -or
        ($WindowHeight -gt 0 -and $noticeHeight -gt $WindowHeight)) {
        throw "Announcement dimensions ${noticeWidth}x${noticeHeight}px exceed the simulated work area."
    }
    [GuardianPreviewNative]::SendMessage($control, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ([GuardianPreviewNative]::FindPreviewNotice($previewProcess.Id) -ne $notice) { throw 'Repeated update clicks created a duplicate announcement window.' }
    $edit = [GuardianPreviewNative]::GetDlgItem($notice, 631)
    $body = [GuardianPreviewNative]::ReadControlText($edit)
    if (-not $body.StartsWith("本版更新`r`n• 自动选择更新线路") -or $body -match '(?m)^[ \t]+[•作者安]') {
        throw "Announcement paragraphs were not normalized to the same left edge: $body"
    }
    $style = [GuardianPreviewNative]::GetWindowStyle($edit, -16).ToInt64()
    if (($style -band 0x800) -eq 0 -or ($style -band 4) -eq 0 -or ($style -band 3) -ne 0) {
        throw 'Announcement body is not a native left-aligned readonly multiline EDIT.'
    }
    $format = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::SendMessageRect($edit, 0xb2, [IntPtr]::Zero, [ref]$format) | Out-Null
    $padding = [int][Math]::Round(12 * $Dpi / 96.0)
    $client = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetClientRect($notice, [ref]$client) | Out-Null
    $verticalPadding = if (($client.Bottom - $client.Top) -lt 360 * $Dpi / 96.0) { [int][Math]::Round(8 * $Dpi / 96.0) } else { $padding }
    if ([Math]::Abs($format.Left - $padding) -gt 1 -or [Math]::Abs($format.Top - $verticalPadding) -gt 1) {
        throw "Announcement padding is [$($format.Left), $($format.Top)]px, expected [$padding, $verticalPadding]px."
    }
    $labelLeft = $null
    $valueLeft = $null
    foreach ($row in 0..3) {
        $label = [GuardianPreviewNative+Rect]::new()
        $value = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($notice, 610+$row), [ref]$label) | Out-Null
        [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($notice, 620+$row), [ref]$value) | Out-Null
        if ($row -eq 0) { $labelLeft=$label.Left; $valueLeft=$value.Left }
        if ($label.Left -ne $labelLeft -or $value.Left -ne $valueLeft -or [Math]::Abs($label.Top-$value.Top) -gt 1) {
            throw "Native announcement label/value row $row does not share its columns and baseline."
        }
    }
    $bodyBounds = [GuardianPreviewNative+Rect]::new()
    $buttonBounds = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetWindowRect($edit, [ref]$bodyBounds) | Out-Null
    [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($notice, 1), [ref]$buttonBounds) | Out-Null
    $textHeight = $format.Bottom - $format.Top
    $firstLine = [GuardianPreviewNative]::SendMessage($edit, 0xd6, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -shr 16
    $secondLineStart = $body.IndexOf("`n") + 1
    $secondLine = [GuardianPreviewNative]::SendMessage($edit, 0xd6, [IntPtr]$secondLineStart, [IntPtr]::Zero).ToInt64() -shr 16
    $lineHeight = $secondLine - $firstLine
    # EDIT may report formatting bounds up to two pixels inside the requested rect.
    if ($bodyBounds.Left -ne $labelLeft -or $bodyBounds.Bottom -gt $buttonBounds.Top -or
        $lineHeight -le 0 -or $textHeight -lt (2 * $lineHeight - 2)) {
        $diagnostic = Join-Path $captureDir ("update-notice-$Dpi-layout-error.png")
        Save-PreviewWindow $notice $diagnostic
        throw "The aligned announcement must fit at least two text lines above its action row. Format: ${textHeight}px; line: ${lineHeight}px; left edges: $($bodyBounds.Left), $labelLeft. Capture: $diagnostic"
    }
    $check = [GuardianPreviewNative]::GetDlgItem($notice, 1)
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    while (-not [GuardianPreviewNative]::IsWindowEnabled($check) -and $elapsed.ElapsedMilliseconds -lt 3500) {
        Start-Sleep -Milliseconds 50
    }
    if (-not [GuardianPreviewNative]::IsWindowEnabled($check)) { throw 'The announcement did not enable retry after the fake check completed.' }
    [GuardianPreviewNative]::SendMessage($notice, 0x28, $check, [IntPtr]1) | Out-Null
    $focusBeforeCheck = [GuardianPreviewNative]::GetDlgCtrlID([GuardianPreviewNative]::PreviewFocus($Window))
    $queuedCheck = [GuardianPreviewNative]::PostMessage($check, 0x100, [IntPtr]13, [IntPtr]::Zero)
    $elapsed.Restart()
    while ([GuardianPreviewNative]::IsWindowEnabled($check) -and $elapsed.ElapsedMilliseconds -lt 700) {
        Start-Sleep -Milliseconds 25
    }
    if ([GuardianPreviewNative]::IsWindowEnabled($check)) {
        $focusAfterCheck = [GuardianPreviewNative]::GetDlgCtrlID([GuardianPreviewNative]::PreviewFocus($Window))
        $phaseAfterCheck = Read-PreviewControl $notice 623
        throw "The announcement retry stayed enabled after Enter. Queued=$queuedCheck, focus $focusBeforeCheck -> $focusAfterCheck, phase='$phaseAfterCheck'."
    }
    if ([GuardianPreviewNative]::GetDlgCtrlID([GuardianPreviewNative]::PreviewFocus($Window)) -ne 2) {
        throw 'Starting the check did not move focus from the disabled retry button to Close.'
    }
    if ((Read-PreviewControl $notice 623) -ne '正在检查更新') { throw 'Retry did not expose its logical progress in the announcement.' }
    [GuardianPreviewNative]::SendMessage($edit, 0xb1, [IntPtr]3, [IntPtr]8) | Out-Null
    [GuardianPreviewNative]::SendMessage($edit, 0xb6, [IntPtr]::Zero, [IntPtr]5) | Out-Null
    $selection = [GuardianPreviewNative]::SendMessage($edit, 0xb0, [IntPtr]::Zero, [IntPtr]::Zero)
    $line = [GuardianPreviewNative]::SendMessage($edit, 0xce, [IntPtr]::Zero, [IntPtr]::Zero)
    $elapsed.Restart()
    while (-not [GuardianPreviewNative]::IsWindowEnabled($check) -and $elapsed.ElapsedMilliseconds -lt 3500) {
        Start-Sleep -Milliseconds 50
    }
    if (-not [GuardianPreviewNative]::IsWindowEnabled($check)) { throw 'Retry did not complete in the announcement.' }
    if ([GuardianPreviewNative]::SendMessage($edit, 0xb0, [IntPtr]::Zero, [IntPtr]::Zero) -ne $selection -or
        [GuardianPreviewNative]::SendMessage($edit, 0xce, [IntPtr]::Zero, [IntPtr]::Zero) -ne $line) {
        throw 'Status refresh reset the selection or scroll position in the announcement.'
    }
    [GuardianPreviewNative]::SendMessage($edit, 0xb1, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::SendMessage($edit, 0xb6, [IntPtr]::Zero, [IntPtr](-10000)) | Out-Null
    $capture = Join-Path $captureDir ("update-notice-$Dpi.png")
    Save-PreviewWindow $notice $capture
    Write-Output $capture
    [GuardianPreviewNative]::SendMessage($notice, 0x28, $edit, [IntPtr]1) | Out-Null
    foreach ($expected in @(1, 2, 631)) {
        $focus = [GuardianPreviewNative]::PreviewFocus($Window)
        [GuardianPreviewNative]::PostMessage($focus, 0x100, [IntPtr]9, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 50
        $focus = [GuardianPreviewNative]::PreviewFocus($Window)
        $actual = [GuardianPreviewNative]::GetDlgCtrlID($focus)
        if ($actual -ne $expected) { throw "Announcement Tab did not reach native control $expected; actual=$actual, focus=$focus." }
    }
    [GuardianPreviewNative]::PostMessage($edit, 0x100, [IntPtr]27, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 100
    if ([GuardianPreviewNative]::IsWindow($notice) -or -not [GuardianPreviewNative]::IsWindow($Window) -or $previewProcess.HasExited) {
        throw 'Closing the announcement affected the main preview or its process.'
    }
    [GuardianPreviewNative]::SendMessage($control, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $reopened = [GuardianPreviewNative]::FindPreviewNotice($previewProcess.Id)
    if ($reopened -eq [IntPtr]::Zero) { throw 'The announcement could not reopen after Escape.' }
    $close = [GuardianPreviewNative]::GetDlgItem($reopened, 2)
    [GuardianPreviewNative]::SendMessage($reopened, 0x28, $close, [IntPtr]1) | Out-Null
    [GuardianPreviewNative]::PostMessage($close, 0x100, [IntPtr]13, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 100
    if ([GuardianPreviewNative]::IsWindow($reopened) -or -not [GuardianPreviewNative]::IsWindow($Window)) { throw 'The native Close button did not close only the announcement.' }
    'Native update notice aligns metadata and text, preserves selection/scroll, and supports Tab, Enter, and Escape.'
}

function Test-PreviewUpdateProgress([IntPtr]$Window) {
    $card = [GuardianPreviewNative]::GetDlgItem($Window, 305)
    [GuardianPreviewNative]::SendMessage($card, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $notice = [GuardianPreviewNative]::FindPreviewNotice($previewProcess.Id)
    if ($notice -eq [IntPtr]::Zero) { throw 'Could not open the update progress preview.' }
    $bar = [GuardianPreviewNative]::GetDlgItem($notice, 640)
    $metrics = [GuardianPreviewNative]::GetDlgItem($notice, 641)
    $edit = [GuardianPreviewNative]::GetDlgItem($notice, 631)
    if ($bar -eq [IntPtr]::Zero -or $metrics -eq [IntPtr]::Zero) {
        throw 'The notice is missing its native download progress bar or transfer numbers.'
    }
    $client = [GuardianPreviewNative+Rect]::new()
    $bounds = [GuardianPreviewNative+Rect]::new()
    $body = [GuardianPreviewNative+Rect]::new()
    $progress = [GuardianPreviewNative+Rect]::new()
    $numbers = [GuardianPreviewNative+Rect]::new()
    $actions = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetClientRect($notice, [ref]$client) | Out-Null
    [GuardianPreviewNative]::MapWindowPoints($notice, [IntPtr]::Zero, [ref]$client, 2) | Out-Null
    [GuardianPreviewNative]::GetWindowRect($notice, [ref]$bounds) | Out-Null
    [GuardianPreviewNative]::GetWindowRect($edit, [ref]$body) | Out-Null
    [GuardianPreviewNative]::GetWindowRect($bar, [ref]$progress) | Out-Null
    [GuardianPreviewNative]::GetWindowRect($metrics, [ref]$numbers) | Out-Null
    [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($notice, 2), [ref]$actions) | Out-Null
    if (($WorkAreaWidth -gt 0 -and $bounds.Right - $bounds.Left -gt $WorkAreaWidth) -or
        ($WindowHeight -gt 0 -and $bounds.Bottom - $bounds.Top -gt $WindowHeight) -or
        $progress.Left -lt $client.Left -or $progress.Right -gt $numbers.Left -or
        $numbers.Right -gt $client.Right -or $numbers.Bottom -gt $body.Top -or
        $body.Bottom -gt $actions.Top -or $actions.Bottom -gt $client.Bottom) {
        throw 'Download controls, notes, or actions do not fit the preview work area.'
    }

    [GuardianPreviewNative]::SendMessage($notice, 0x28, $edit, [IntPtr]1) | Out-Null
    [GuardianPreviewNative]::SendMessage($edit, 0xb1, [IntPtr]3, [IntPtr]8) | Out-Null
    [GuardianPreviewNative]::SendMessage($edit, 0xb6, [IntPtr]::Zero, [IntPtr]5) | Out-Null
    $selection = [GuardianPreviewNative]::SendMessage($edit, 0xb0, [IntPtr]::Zero, [IntPtr]::Zero)
    $line = [GuardianPreviewNative]::SendMessage($edit, 0xce, [IntPtr]::Zero, [IntPtr]::Zero)
    foreach ($stage in @(
        @{ Name='select'; Phase='正在测试下载线路'; Source='流年 App · CF CDN'; Card='正在选线' },
        @{ Name='start'; Phase='正在下载安装包'; Source='流年 App · CF CDN · 多线程'; Card='正在下载'; Position=250; Metrics='25%  ·  8.0 MiB / 32.0 MiB' },
        @{ Name='advance'; Phase='正在下载安装包'; Source='流年 App · CF CDN · 多线程'; Card='正在下载'; Position=625; Metrics='62.5%  ·  20.0 MiB / 32.0 MiB' },
        @{ Name='retry'; Phase='当前线路失败，正在切换'; Source='流年 App · CF CDN · 多线程'; Card='正在重试' },
        @{ Name='restart'; Phase='已切换线路，正在重新下载'; Source='GitHub 镜像 · 单线程'; Card='正在重试'; Position=125; Metrics='12.5%  ·  4.0 MiB / 32.0 MiB' },
        @{ Name='verify'; Phase='正在校验安装包'; Source='GitHub 镜像 · 单线程'; Card='正在校验' }
    )) {
        [IO.File]::WriteAllText($updateFixturePath, $stage.Name)
        [GuardianPreviewNative]::SendMessage($notice, 0x113, [IntPtr]1, [IntPtr]::Zero) | Out-Null
        [GuardianPreviewNative]::SendMessage($Window, 0x113, [IntPtr]1, [IntPtr]::Zero) | Out-Null
        $phase = Read-PreviewControl $notice 623
        $source = Read-PreviewControl $notice 622
        $caption = [GuardianPreviewNative]::ReadControlText($card)
        if ($phase -ne $stage.Phase -or $source -ne $stage.Source -or -not $caption.Contains($stage.Card)) {
            throw "Incorrect phase, source, or sidebar status in $($stage.Name): '$phase', '$source', '$caption'."
        }
        if ($stage.ContainsKey('Position')) {
            $position = [GuardianPreviewNative]::SendMessage($bar, 0x408, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
            if (-not [GuardianPreviewNative]::IsWindowVisible($bar) -or -not [GuardianPreviewNative]::IsWindowVisible($metrics) -or
                $position -ne $stage.Position -or [GuardianPreviewNative]::ReadControlText($metrics) -ne $stage.Metrics) {
                throw "Transfer stage $($stage.Name) did not expose its measured progress."
            }
        } elseif ([GuardianPreviewNative]::IsWindowVisible($bar) -or [GuardianPreviewNative]::IsWindowVisible($metrics)) {
            throw "Non-transfer stage $($stage.Name) displays a percentage or progress bar."
        }
        $currentBody = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($edit, [ref]$currentBody) | Out-Null
        if ($currentBody.Top -ne $body.Top -or $currentBody.Bottom -ne $body.Bottom -or
            [GuardianPreviewNative]::SendMessage($edit, 0xb0, [IntPtr]::Zero, [IntPtr]::Zero) -ne $selection -or
            [GuardianPreviewNative]::SendMessage($edit, 0xce, [IntPtr]::Zero, [IntPtr]::Zero) -ne $line -or
            [GuardianPreviewNative]::PreviewFocus($Window) -ne $edit) {
            throw "The announcement lost its layout, selection, scroll position, or keyboard focus in $($stage.Name)."
        }
        if ($stage.Name -in @('advance', 'retry', 'restart', 'verify')) {
            $capture = Join-Path $captureDir ("update-progress-$Dpi-$($stage.Name).png")
            # Give the themed Win32 progress bar time to paint its new position.
            if ($stage.ContainsKey('Position')) { Start-Sleep -Milliseconds 550 }
            [GuardianPreviewNative]::SendMessage($edit, 0xb1, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            [GuardianPreviewNative]::SendMessage($edit, 0xb6, [IntPtr]::Zero, [IntPtr](-$line.ToInt32())) | Out-Null
            Save-PreviewWindow $notice $capture
            [GuardianPreviewNative]::SendMessage($edit, 0xb1, [IntPtr]3, [IntPtr]8) | Out-Null
            [GuardianPreviewNative]::SendMessage($edit, 0xb6, [IntPtr]::Zero, $line) | Out-Null
            Write-Output $capture
        }
    }
    'Native download progress reports measured bytes, resets on retry, and keeps announcement notes in place through verification.'
}

function Test-PreviewTrayActivation([IntPtr]$Window) {
    # Keep the isolated off-screen preview disabled while activating it: Windows
    # cannot give a disabled top-level window foreground keyboard focus.
    [GuardianPreviewNative]::EnableWindow($Window, $false) | Out-Null
    try {
        foreach ($state in @('hidden startup', 'minimized', 'closed to tray', 'already visible')) {
            if ($state -eq 'minimized') {
                [GuardianPreviewNative]::ShowWindow($Window, 7) | Out-Null
                if (-not [GuardianPreviewNative]::IsIconic($Window)) { throw 'Could not prepare minimized preview.' }
            } elseif ($state -eq 'closed to tray') {
                [GuardianPreviewNative]::SendMessage($Window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            }
            # Exercise the actual application callback dispatched by Shell for
            # a tray double-click, without moving or clicking the user's mouse.
            [GuardianPreviewNative]::SendMessage($Window, 0x8001, [IntPtr]1, [IntPtr]0x203) | Out-Null
            if (-not [GuardianPreviewNative]::IsWindowVisible($Window) -or [GuardianPreviewNative]::IsIconic($Window)) {
                throw "Tray double-click did not restore the panel from '$state'."
            }
            "Tray double-click restored panel from: $state"
        }
        [GuardianPreviewNative]::ShowWindow($Window, 7) | Out-Null
        [GuardianPreviewNative]::SendMessage($Window, 0x111, [IntPtr]202, [IntPtr]::Zero) | Out-Null
        if (-not [GuardianPreviewNative]::IsWindowVisible($Window) -or [GuardianPreviewNative]::IsIconic($Window)) {
            throw 'Settings menu did not restore the minimized panel.'
        }
        'Settings menu restored minimized panel.'
    } finally {
        [GuardianPreviewNative]::EnableWindow($Window, $true) | Out-Null
    }
}

function Read-PreviewControl([IntPtr]$Window, [int]$Id) {
    $control = [GuardianPreviewNative]::GetDlgItem($Window, $Id)
    if ($control -eq [IntPtr]::Zero) { throw "Preview control $Id does not exist." }
    return [GuardianPreviewNative]::ReadControlText($control)
}

function Get-PreviewStatusDiagnostics([IntPtr]$Window) {
    $overviewNav = [GuardianPreviewNative]::GetDlgItem($Window, 301)
    [GuardianPreviewNative]::SendMessage($overviewNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $statusEdit = [GuardianPreviewNative]::GetDlgItem($Window, 110)
    $lineCount = [GuardianPreviewNative]::SendMessage($statusEdit, 0xba, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    $secondLine = [GuardianPreviewNative]::SendMessage($statusEdit, 0xbb, [IntPtr]1, [IntPtr]::Zero).ToInt64()
    $style = [GuardianPreviewNative]::GetWindowStyle($statusEdit, -16).ToInt64()
    $format = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::SendMessageRect($statusEdit, 0xb2, [IntPtr]::Zero, [ref]$format) | Out-Null
    $client = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetClientRect($statusEdit, [ref]$client) | Out-Null
    "Fake status EDIT: lines=$lineCount, secondLineIndex=$secondLine, style=0x$($style.ToString('X')), WS_VSCROLL=$([bool]($style -band 0x200000)), formatting=[$($format.Left),$($format.Top),$($format.Right),$($format.Bottom)], client=[$($client.Left),$($client.Top),$($client.Right),$($client.Bottom)]."
}

function Test-PreviewUnbrokenAdapter([IntPtr]$Window) {
    $settingsNav = [GuardianPreviewNative]::GetDlgItem($Window, 302)
    [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $expected = 'preview-adapter-' + ('W' * 200) + '-WHUT-ISP'
    $detail = Read-PreviewControl $Window 134
    $normalized = $detail -replace '[\r\n]', ''
    $rect = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($Window, 134), [ref]$rect) | Out-Null
    $height = $rect.Bottom - $rect.Top
    $oneLine = [int][Math]::Ceiling(24 * $Dpi / 96.0)
    if (-not $normalized.Contains($expected) -or $height -le $oneLine) {
        throw "Unbroken fake adapter name is not fully available in a multiline detail: expectedLength=$($expected.Length), normalizedDetailLength=$($normalized.Length), height=$height, oneLineThreshold=$oneLine."
    }
    "Unbroken fake adapter name is fully available in the native multiline detail: $($expected.Length) ASCII characters, height=$height."
}

function Test-PreviewValidation([IntPtr]$Window) {
    $report = [Collections.Generic.List[string]]::new()
    $settingsNav = [GuardianPreviewNative]::GetDlgItem($Window, 302)
    [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ([GuardianPreviewNative]::GetDlgItem($Window, 131) -eq [IntPtr]::Zero) {
        throw 'Inline validation feedback control 131 is missing; no Save was attempted.'
    }
    $combo = [GuardianPreviewNative]::GetDlgItem($Window, 101)
    $username = [GuardianPreviewNative]::GetDlgItem($Window, 102)
    $password = [GuardianPreviewNative]::GetDlgItem($Window, 103)
    $consent = [GuardianPreviewNative]::GetDlgItem($Window, 104)
    $save = [GuardianPreviewNative]::GetDlgItem($Window, 106)

    # Every scenario is invalid before read_password/snapshot_autostart in
    # save_form. Never add an all-valid Save here: the native form owns those
    # preflight calls, even though the preview has no controller or worker.
    foreach ($scenario in @(
        @{ Name='missing adapter'; Adapter=-1; User='demo-campus'; Password='fake-preview-draft-only'; Consent=1; Focus=101; Error='网卡' },
        @{ Name='missing username'; Adapter=0; User=''; Password='fake-preview-draft-only'; Consent=1; Focus=102; Error='账号' },
        @{ Name='missing HTTP consent'; Adapter=0; User='demo-campus'; Password='fake-preview-draft-only'; Consent=0; Focus=104; Error='HTTP' },
        @{ Name='changed username needs password'; Adapter=0; User='demo-edited'; Password=''; Consent=1; Focus=103; Error='密码' }
    )) {
        [GuardianPreviewNative]::SendMessage($combo, 0x14e, [IntPtr]$scenario.Adapter, [IntPtr]::Zero) | Out-Null
        $setUser = [GuardianPreviewNative]::SetControlText($username, $scenario.User)
        $setPassword = [GuardianPreviewNative]::SetControlText($password, $scenario.Password)
        [GuardianPreviewNative]::SendMessage($consent, 0xf1, [IntPtr]$scenario.Consent, [IntPtr]::Zero) | Out-Null
        $beforeUser = Read-PreviewControl $Window 102
        $beforePasswordLength = [GuardianPreviewNative]::SendMessage($password, 0xe, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        $beforeAdapter = [GuardianPreviewNative]::SendMessage($combo, 0x147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        $beforeConsent = [GuardianPreviewNative]::SendMessage($consent, 0xf0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        $adapterDetail = [GuardianPreviewNative]::GetDlgItem($Window, 134)
        $networkTitle = [GuardianPreviewNative]::GetDlgItem($Window, 125)
        $detailBounds = [GuardianPreviewNative+Rect]::new()
        $networkBounds = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($adapterDetail, [ref]$detailBounds) | Out-Null
        [GuardianPreviewNative]::GetWindowRect($networkTitle, [ref]$networkBounds) | Out-Null
        $detailOffset = $detailBounds.Top - $networkBounds.Top
        if ($beforeUser -ne $scenario.User -or $beforePasswordLength -ne $scenario.Password.Length -or $beforeAdapter -ne $scenario.Adapter -or $beforeConsent -ne $scenario.Consent) {
            throw "Fake preconditions for '$($scenario.Name)' were not applied; no Save attempted. User='$beforeUser'; passwordLength=$beforePasswordLength; adapter=$beforeAdapter; consent=$beforeConsent; setUser=$setUser; setPassword=$setPassword."
        }
        [GuardianPreviewNative]::SendMessage($save, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 70
        $feedback = Read-PreviewControl $Window 131
        if ($feedback -notmatch $scenario.Error) { throw "Inline feedback for '$($scenario.Name)' did not explain the invalid field: $feedback" }
        [GuardianPreviewNative]::GetWindowRect($adapterDetail, [ref]$detailBounds) | Out-Null
        [GuardianPreviewNative]::GetWindowRect($networkTitle, [ref]$networkBounds) | Out-Null
        if ($detailBounds.Top - $networkBounds.Top -ne $detailOffset) {
            throw "Validation '$($scenario.Name)' displaced the adapter detail from its card."
        }
        $focus = [GuardianPreviewNative]::PreviewFocus($Window)
        $focusId = [GuardianPreviewNative]::GetDlgCtrlID($focus)
        $afterPasswordLength = [GuardianPreviewNative]::SendMessage($password, 0xe, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
        if ($focusId -ne $scenario.Focus) { throw "Validation '$($scenario.Name)' focused $focusId instead of $($scenario.Focus). Feedback='$feedback'; fakeUser='$(Read-PreviewControl $Window 102)'; passwordLength=$afterPasswordLength; beforeFakeUser='$beforeUser'; beforePasswordLength=$beforePasswordLength." }
        if ((Read-PreviewControl $Window 102) -ne $scenario.User -or $afterPasswordLength -ne $scenario.Password.Length) {
            throw "Validation '$($scenario.Name)' discarded an unsaved fake field."
        }
        if (-not [GuardianPreviewNative]::IsWindowVisible($save)) { throw "Validation '$($scenario.Name)' unexpectedly left the settings page." }
        if ($WindowHeight -gt 0) {
            $client = [GuardianPreviewNative+Rect]::new()
            $error = [GuardianPreviewNative+Rect]::new()
            $field = [GuardianPreviewNative+Rect]::new()
            [GuardianPreviewNative]::GetClientRect($Window, [ref]$client) | Out-Null
            [GuardianPreviewNative]::MapWindowPoints($Window, [IntPtr]::Zero, [ref]$client, 2) | Out-Null
            [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($Window, 131), [ref]$error) | Out-Null
            [GuardianPreviewNative]::GetWindowRect($focus, [ref]$field) | Out-Null
            if ($error.Top -lt $client.Top -or $error.Bottom -gt $client.Bottom -or
                $field.Top -lt $client.Top -or $field.Bottom -gt $client.Bottom) {
                throw "Validation '$($scenario.Name)' did not keep the message and its focused field visible in the short window."
            }
        }
        $report.Add("Inline validation '$($scenario.Name)' explains the error, focuses field $focusId, and preserves fake form input.")
    }
    # Leave the last invalid scenario in place for the validation screenshot.
    # The isolated process closes immediately after capture.
    return $report
}

function Test-PreviewAnalytics([IntPtr]$Window) {
    $settingsNav = [GuardianPreviewNative]::GetDlgItem($Window, 302)
    $overviewNav = [GuardianPreviewNative]::GetDlgItem($Window, 301)
    $analytics = [GuardianPreviewNative]::GetDlgItem($Window, 138)
    $disclosure = [GuardianPreviewNative]::GetDlgItem($Window, 139)
    if ($analytics -eq [IntPtr]::Zero -or $disclosure -eq [IntPtr]::Zero) {
        throw 'The settings page needs an analytics consent control and disclosure.'
    }
    $username = [GuardianPreviewNative]::GetDlgItem($Window, 102)
    $httpConsent = [GuardianPreviewNative]::GetDlgItem($Window, 104)
    [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ([GuardianPreviewNative]::SendMessage($analytics, 0xf0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) {
        throw 'Analytics consent must start unchecked.'
    }
    $text = [GuardianPreviewNative]::ReadControlText($disclosure)
    if ($text -notmatch '每天最多.*一次' -or $text -notmatch '随机安装标识' -or
        $text -notmatch '版本' -or $text -notmatch '清除.*标识') {
        throw "Analytics disclosure is incomplete: $text"
    }
    [GuardianPreviewNative]::SetControlText($username, 'demo-consent-draft') | Out-Null
    [GuardianPreviewNative]::SendMessage($httpConsent, 0xf1, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    foreach ($consentState in @(1, 0)) {
        [GuardianPreviewNative]::SendMessage($analytics, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        [GuardianPreviewNative]::SendMessage($analytics, 0xf1, [IntPtr](1 - $consentState), [IntPtr]::Zero) | Out-Null
        [GuardianPreviewNative]::SendMessage($overviewNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        [GuardianPreviewNative]::SendMessage($settingsNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        if ([GuardianPreviewNative]::SendMessage($analytics, 0xf0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne $consentState) {
            throw "Analytics consent did not apply independently with state $consentState."
        }
    }
    if ((Read-PreviewControl $Window 102) -ne 'demo-consent-draft' -or
        [GuardianPreviewNative]::SendMessage($httpConsent, 0xf0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) {
        throw 'Analytics consent changed the unsaved authentication form.'
    }
    [GuardianPreviewNative]::SetControlText($username, 'demo-campus') | Out-Null
    [GuardianPreviewNative]::SendMessage($httpConsent, 0xf1, [IntPtr]1, [IntPtr]::Zero) | Out-Null

    # Exercise the tray command off screen without foreground activation.
    [GuardianPreviewNative]::SendMessage($overviewNav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [GuardianPreviewNative]::EnableWindow($Window, $false) | Out-Null
    try {
        [GuardianPreviewNative]::SendMessage($Window, 0x111, [IntPtr]206, [IntPtr]::Zero) | Out-Null
    } finally {
        [GuardianPreviewNative]::EnableWindow($Window, $true) | Out-Null
    }
    $client = [GuardianPreviewNative+Rect]::new()
    $checkBounds = [GuardianPreviewNative+Rect]::new()
    $disclosureBounds = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetClientRect($Window, [ref]$client) | Out-Null
    [GuardianPreviewNative]::MapWindowPoints($Window, [IntPtr]::Zero, [ref]$client, 2) | Out-Null
    [GuardianPreviewNative]::GetWindowRect($analytics, [ref]$checkBounds) | Out-Null
    [GuardianPreviewNative]::GetWindowRect($disclosure, [ref]$disclosureBounds) | Out-Null
    if (-not [GuardianPreviewNative]::IsWindowVisible($analytics) -or
        $checkBounds.Top -lt $client.Top -or $disclosureBounds.Bottom -gt $client.Bottom) {
        throw 'The tray command did not bring the analytics controls and disclosure into view.'
    }
    if ([GuardianPreviewNative]::SendMessage($analytics, 0xf0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) {
        throw 'Opening the analytics setting from the tray granted consent.'
    }
    'Analytics starts off, can be enabled and disabled independently, and opens from the tray without granting consent.'
}

function Test-PreviewDiagnostics([IntPtr]$Window) {
    $overview = [GuardianPreviewNative]::GetDlgItem($Window, 301)
    [GuardianPreviewNative]::SendMessage($overview, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $summary = Read-PreviewControl $Window 141
    foreach ($stage in @('网络接入', '校园网认证', '运营商拨号', 'IPv4 外网')) {
        if (-not $summary.Contains($stage)) { throw "Connection diagnostics do not expose the $stage stage." }
    }
    if ($Status -eq 'dual' -and ($summary -notmatch 'Wi-Fi' -or $summary -notmatch '以太网')) {
        throw 'Connection diagnostics are missing the second network.'
    }
    $check = [GuardianPreviewNative]::GetDlgItem($Window, 207)
    [GuardianPreviewNative]::SendMessage($check, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    while ((Read-PreviewControl $Window 141) -notmatch 'IPv4 外网 可访问' -and $elapsed.ElapsedMilliseconds -lt 3500) {
        Start-Sleep -Milliseconds 50
    }
    if ((Read-PreviewControl $Window 141) -notmatch 'IPv4 外网 可访问') {
        throw 'The fake external result did not refresh independently of the guardian status.'
    }

    $toggle = [GuardianPreviewNative]::GetDlgItem($Window, 142)
    $copy = [GuardianPreviewNative]::GetDlgItem($Window, 143)
    $report = [GuardianPreviewNative]::GetDlgItem($Window, 144)
    if ([GuardianPreviewNative]::IsWindowVisible($report)) { throw 'Recovery history should start collapsed.' }
    [GuardianPreviewNative]::SendMessage($toggle, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $style = [GuardianPreviewNative]::GetWindowStyle($report, -16).ToInt64()
    if (-not [GuardianPreviewNative]::IsWindowVisible($report) -or ($style -band 0x804) -ne 0x804 -or
        -not (Read-PreviewControl $Window 144).StartsWith("最近恢复记录`r`n")) {
        throw 'Recovery history did not expand in a read-only multiline control.'
    }
    [GuardianPreviewNative]::SendMessage($report, 0xb6, [IntPtr]::Zero, [IntPtr]12) | Out-Null
    if ([GuardianPreviewNative]::SendMessage($report, 0xce, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -le 0) {
        throw 'The bounded recovery history did not scroll.'
    }
    [GuardianPreviewNative]::SendMessage($copy, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ((Read-PreviewControl $Window 145) -ne '演示诊断已生成。') { throw 'The preview-safe copy action did not acknowledge the report.' }

    if ([GuardianPreviewNative]::PreviewFocus($Window) -ne $copy) { throw 'The copy button did not receive native focus.' }
    [GuardianPreviewNative]::PostMessage($copy, 0x100, [IntPtr]9, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 100
    if ([GuardianPreviewNative]::PreviewFocus($Window) -ne $report) { throw 'Tab from Copy did not focus recovery history.' }
    if ([GuardianPreviewNative]::SendMessage($report, 0xb0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne 0) {
        throw 'Tab into recovery history selected the report text.'
    }
    $bounds = [GuardianPreviewNative+Rect]::new()
    $client = [GuardianPreviewNative+Rect]::new()
    [GuardianPreviewNative]::GetWindowRect($report, [ref]$bounds) | Out-Null
    [GuardianPreviewNative]::GetClientRect($Window, [ref]$client) | Out-Null
    [GuardianPreviewNative]::MapWindowPoints($Window, [IntPtr]::Zero, [ref]$client, 2) | Out-Null
    if ($bounds.Top -lt $client.Top -or $bounds.Bottom -gt $client.Bottom) {
        throw 'Tab focus did not reveal the entire recovery history control.'
    }
    [GuardianPreviewNative]::SendMessage($report, 0xb6, [IntPtr]::Zero, [IntPtr](-100)) | Out-Null

    $capture = Join-Path $captureDir ("overview-diagnostics-$Dpi-$Status.png")
    Save-PreviewWindow $Window $capture
    Write-Output $capture
    [GuardianPreviewNative]::SendMessage($toggle, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ([GuardianPreviewNative]::IsWindowVisible($report)) { throw 'Recovery history did not collapse.' }
    'Connection diagnostics expose each stage, refresh the fake external result, support keyboard and scroll access to history, and acknowledge preview-safe copying.'
}

function Save-PreviewWindow([IntPtr]$Window, [string]$Destination) {
    $rect = [GuardianPreviewNative+Rect]::new()
    $gotRect = if ($ClientOnly) { [GuardianPreviewNative]::GetClientRect($Window, [ref]$rect) } else { [GuardianPreviewNative]::GetWindowRect($Window, [ref]$rect) }
    if (-not $gotRect) { throw 'Could not obtain preview window bounds.' }
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $dc = [IntPtr]::Zero
    try {
        [GuardianPreviewNative]::RedrawWindow($Window, [IntPtr]::Zero, [IntPtr]::Zero, 0x185) | Out-Null
        $dc = $graphics.GetHdc()
        $printFlags = if ($ClientOnly) { 3 } else { 2 }
        if (-not [GuardianPreviewNative]::PrintWindow($Window, $dc, $printFlags)) { throw 'PrintWindow rejected the preview capture.' }
        $graphics.ReleaseHdc($dc)
        $dc = [IntPtr]::Zero
        $bitmap.Save($Destination, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        if ($dc -ne [IntPtr]::Zero) { $graphics.ReleaseHdc($dc) }
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

$previewProcess = $null
$previewWindow = [IntPtr]::Zero
$priorDpi = [Environment]::GetEnvironmentVariable('CAMPUS_UI_PREVIEW_DPI')
$priorWidth = [Environment]::GetEnvironmentVariable('CAMPUS_UI_PREVIEW_WORK_WIDTH')
$priorNoticeHeight = [Environment]::GetEnvironmentVariable('CAMPUS_UI_PREVIEW_NOTICE_HEIGHT')
$priorScrollMode = [Environment]::GetEnvironmentVariable('CAMPUS_UI_PREVIEW_SCROLL_MODE')
$priorStatus = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_STATUS')
$priorUpdateFixture = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_UPDATE_FIXTURE')
$updateFixturePath = Join-Path $captureDir 'update-stage.txt'
$priorError = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_ERROR')
$priorLongAdapter = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_LONG_ADAPTER')
$priorUnbrokenAdapter = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_UNBROKEN_ADAPTER')
$errorPath = Join-Path $captureDir 'error.txt'
$priorDpiContext = [GuardianPreviewNative]::SetThreadDpiAwarenessContext([IntPtr](-4))
try {
    $env:CAMPUS_UI_PREVIEW_DPI = [string]$Dpi
    $env:CAMPUS_UI_PREVIEW_WORK_WIDTH = [string]$WorkAreaWidth
    if ($VerifyUpdateNotice -or $VerifyUpdateProgress) { $env:CAMPUS_UI_PREVIEW_NOTICE_HEIGHT = [string]$WindowHeight }
    if ($VerifyUpdateProgress) { $env:CAMPUS_PREVIEW_UPDATE_FIXTURE = $updateFixturePath }
    if ($VerifyPageScrollbar) {
        $env:CAMPUS_UI_PREVIEW_SCROLL_MODE = if ($AlwaysShowPageScrollbar) { 'always' } else { 'auto' }
    }
    $env:CAMPUS_PREVIEW_STATUS = $Status
    $env:CAMPUS_PREVIEW_ERROR = $errorPath
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_LONG_ADAPTER', $(if ($LongAdapter) { '1' } else { $null }))
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_UNBROKEN_ADAPTER', $(if ($UnbrokenAdapter) { '1' } else { $null }))
    $previewProcess = Start-Process -FilePath $previewExe -WindowStyle Hidden -PassThru
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        if ($previewProcess.HasExited) {
            $errorDetails = if (Test-Path -LiteralPath $errorPath) { Get-Content -LiteralPath $errorPath -Raw } else { 'No error file.' }
            throw "Preview exited before creating a window: $errorDetails"
        }
        $previewWindow = [GuardianPreviewNative]::FindPreviewWindow($previewProcess.Id)
        if ($previewWindow -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($previewWindow -eq [IntPtr]::Zero) { throw 'Preview window was not found within five seconds.' }
    if ($WorkAreaWidth -gt 0) {
        $widthRect = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($previewWindow, [ref]$widthRect) | Out-Null
        $previewWidth = $widthRect.Right - $widthRect.Left
        if ($previewWidth -gt $WorkAreaWidth) { throw "Preview window width $previewWidth exceeds its $WorkAreaWidth px work area." }
        Write-Output "Preview window width $previewWidth px fits the $WorkAreaWidth px work area at $Dpi DPI."
    }
    # No focus or real screen capture. A real window must have WS_VISIBLE for
    # every child to render via WM_PRINT; keep it beyond all monitor bounds.
    [GuardianPreviewNative]::SetWindowPos($previewWindow, [IntPtr]::Zero, -20000, -20000, 0, 0, 0x15) | Out-Null
    if ($VerifyTrayActivation) {
        Test-PreviewTrayActivation $previewWindow | Write-Output
    }
    [GuardianPreviewNative]::ShowWindow($previewWindow, 4) | Out-Null
    if ($WindowHeight -gt 0) {
        $rect = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($previewWindow, [ref]$rect) | Out-Null
        [GuardianPreviewNative]::SetWindowPos($previewWindow, [IntPtr]::Zero, -20000, -20000, $rect.Right-$rect.Left, $WindowHeight, 0x14) | Out-Null
    }
    Start-Sleep -Milliseconds 200
    $pages = if ($Page -eq 'all') { @('overview', 'settings') } else { @($Page) }
    foreach ($capturePage in $pages) {
        $navId = if ($capturePage -eq 'overview') { 301 } else { 302 }
        $nav = [GuardianPreviewNative]::GetDlgItem($previewWindow, $navId)
        if ($nav -ne [IntPtr]::Zero) {
            [GuardianPreviewNative]::SendMessage($nav, 0xf5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        } elseif ($capturePage -eq 'overview') {
            Write-Warning 'This UI source has no overview navigation yet; captured the available page.'
        }
        Start-Sleep -Milliseconds 200
        if ($capturePage -eq 'overview' -and $Status -eq 'dual' -and $WindowHeight -ge 650) {
            $bounds = [GuardianPreviewNative+Rect]::new()
            $action = [GuardianPreviewNative+Rect]::new()
            [GuardianPreviewNative]::GetClientRect($previewWindow, [ref]$bounds) | Out-Null
            [GuardianPreviewNative]::MapWindowPoints($previewWindow, [IntPtr]::Zero, [ref]$bounds, 2) | Out-Null
            [GuardianPreviewNative]::GetWindowRect([GuardianPreviewNative]::GetDlgItem($previewWindow, 201), [ref]$action) | Out-Null
            if ($action.Bottom -gt $bounds.Bottom) { throw 'The overview check action is clipped before scrolling.' }
            Write-Output 'The overview check action is fully visible on the short screen.'
        }
        if ($ScrollBottom) {
            [GuardianPreviewNative]::SendMessage($previewWindow, 0x115, [IntPtr]7, [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 100
        }
        $bottomSuffix = if ($ScrollBottom) { '-bottom' } else { '' }
        $clientSuffix = if ($ClientOnly) { '-client' } else { '' }
        $capturePath = Join-Path $captureDir ("$capturePage-$Dpi-$Status$bottomSuffix$clientSuffix.png")
        Save-PreviewWindow $previewWindow $capturePath
        Write-Output $capturePath
    }
    if ($VerifyNavigation) {
        $report = Test-PreviewNavigation $previewWindow
        $reportPath = Join-Path $captureDir 'navigation-check.txt'
        [IO.File]::WriteAllLines($reportPath, $report, [Text.UTF8Encoding]::new($false))
        Write-Output $reportPath
        $report | Write-Output
    }
    if ($VerifyAuthor) {
        Test-PreviewAuthor $previewWindow | Write-Output
    }
    if ($VerifyUpdateCard) {
        Test-PreviewUpdateCard $previewWindow | Write-Output
    }
    if ($VerifyUpdateNotice) {
        Test-PreviewUpdateNotice $previewWindow | Write-Output
    }
    if ($VerifyUpdateProgress) {
        Test-PreviewUpdateProgress $previewWindow | Write-Output
    }
    if ($VerifyPageScrollbar) {
        Test-PreviewPageScrollbar $previewWindow | Write-Output
    }
    if ($UnbrokenAdapter) {
        Test-PreviewUnbrokenAdapter $previewWindow | Write-Output
    }
    if ($VerifyValidation) {
        $report = Test-PreviewValidation $previewWindow
        $reportPath = Join-Path $captureDir 'validation-check.txt'
        [IO.File]::WriteAllLines($reportPath, $report, [Text.UTF8Encoding]::new($false))
        Write-Output $reportPath
        $report | Write-Output
        $validationCapture = Join-Path $captureDir ("settings-validation-$Dpi.png")
        Save-PreviewWindow $previewWindow $validationCapture
        Write-Output $validationCapture
    }
    if ($VerifyAnalytics) {
        Test-PreviewAnalytics $previewWindow | Write-Output
        $analyticsCapture = Join-Path $captureDir ("settings-analytics-$Dpi.png")
        Save-PreviewWindow $previewWindow $analyticsCapture
        Write-Output $analyticsCapture
    }
    if ($VerifyDiagnostics) {
        Test-PreviewDiagnostics $previewWindow | Write-Output
    }
    if ($DiagnoseStatus) {
        Get-PreviewStatusDiagnostics $previewWindow | Write-Output
    }
} finally {
    [Environment]::SetEnvironmentVariable('CAMPUS_UI_PREVIEW_DPI', $priorDpi)
    [Environment]::SetEnvironmentVariable('CAMPUS_UI_PREVIEW_WORK_WIDTH', $priorWidth)
    [Environment]::SetEnvironmentVariable('CAMPUS_UI_PREVIEW_NOTICE_HEIGHT', $priorNoticeHeight)
    [Environment]::SetEnvironmentVariable('CAMPUS_UI_PREVIEW_SCROLL_MODE', $priorScrollMode)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_STATUS', $priorStatus)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_UPDATE_FIXTURE', $priorUpdateFixture)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_ERROR', $priorError)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_LONG_ADAPTER', $priorLongAdapter)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_UNBROKEN_ADAPTER', $priorUnbrokenAdapter)
    if ($previewProcess -and -not $previewProcess.HasExited) {
        $notice = [GuardianPreviewNative]::FindPreviewNotice($previewProcess.Id)
        if ($notice -ne [IntPtr]::Zero) {
            [GuardianPreviewNative]::PostMessage($notice, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        }
        if ($previewWindow -ne [IntPtr]::Zero) {
            # The application handles this command by destroying its own window,
            # removing its own tray icon, and leaving its UI loop.
            [GuardianPreviewNative]::PostMessage($previewWindow, 0x111, [IntPtr]204, [IntPtr]::Zero) | Out-Null
        }
        if (-not $previewProcess.WaitForExit(3000)) {
            if ($previewWindow -ne [IntPtr]::Zero) { [GuardianPreviewNative]::RemovePreviewTray($previewWindow) }
            $previewProcess.Kill()
            $previewProcess.WaitForExit()
        }
    }
    if ($previewProcess) { $previewProcess.Dispose() }
    if ($priorDpiContext -ne [IntPtr]::Zero) { [GuardianPreviewNative]::SetThreadDpiAwarenessContext($priorDpiContext) | Out-Null }
}
