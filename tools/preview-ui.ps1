<#
.SYNOPSIS
Build and capture an isolated native UI with fake data, without running the worker.
.DESCRIPTION
The preview lives off screen and does not receive foreground focus. Validation
checks may click Save only with deliberately invalid fake fields that return
before any credential, registry, or controller transaction. It never clicks
Delete, the portal link, or a network operation. Captures use PrintWindow rather
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
    [switch]$VerifyTrayActivation,
    [switch]$VerifyValidation,
    [switch]$LongAdapter,
    [switch]$UnbrokenAdapter,
    [switch]$DiagnoseStatus
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildDir = Join-Path $workspace 'target\ui-review'
New-Item -ItemType Directory -Path $buildDir -Force | Out-Null
$captureDir = Join-Path ([IO.Path]::GetTempPath()) ('campus-guardian-ui-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $captureDir -Force | Out-Null

if (-not $ResourceLibrary) {
    $ResourceLibrary = Get-ChildItem -LiteralPath (Join-Path $workspace 'target') -Filter 'guardian.lib' -Recurse |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $ResourceLibrary -or -not (Test-Path -LiteralPath $ResourceLibrary -PathType Leaf)) {
    throw 'No compiled guardian.lib found. Build the application once before using this preview.'
}

$previewSource = Join-Path $buildDir 'preview.rs'
$previewExe = Join-Path $buildDir ('preview-' + (Get-Date -Format 'yyyyMMddHHmmssfff') + '.exe')
$uiSource = (Join-Path $workspace 'src\windows_ui.rs').Replace('\', '/')
$previewText = @'
#![windows_subsystem = "windows"]
#![allow(dead_code, unused_imports)]
#[path = "__UI_SOURCE__"] mod windows_ui;
use std::sync::{atomic::AtomicU64, mpsc, Arc, Mutex};
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
        $allowed = if ($navigationPage -eq 'overview') { @(301,302,303,304,110,201,203) } else { @(301,302,303,304,101,135,102,103,104,105,106,107) }
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
        if (-not [GuardianPreviewNative]::GetScrollInfo($Window, 1, [ref]$scroll)) { throw 'Could not read the isolated short-screen scroll range.' }
        $expectedBottom = [Math]::Max($scroll.Min, $scroll.Max - [int]$scroll.Page + 1)
        # Manual scrolling must remain possible while a form field has focus.
        # This does not move the pointer, type, or activate the real desktop.
        [GuardianPreviewNative]::SendMessage($Window, 0x115, [IntPtr]7, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 150
        $after = [GuardianPreviewNative+Rect]::new()
        [GuardianPreviewNative]::GetWindowRect($username, [ref]$after) | Out-Null
        [GuardianPreviewNative]::GetScrollInfo($Window, 1, [ref]$scroll) | Out-Null
        $focusId = [GuardianPreviewNative]::GetDlgCtrlID([GuardianPreviewNative]::PreviewFocus($Window))
        if ($focusId -ne 102 -or $scroll.Position -ne $expectedBottom) {
            throw "Manual short-screen scroll snapped back to the focused field: focus=$focusId, scrollPosition=$($scroll.Position), requestedBottom=$expectedBottom, fieldTopBefore=$($before.Top), fieldTopAfter=$($after.Top)."
        }
        $report.Add('Short-screen manual scrolling remains at the requested position while the username field retains focus.')
    }
    return $report
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
    foreach ($entry in @(
        @{ Id=303; Label='Liunian520'; Role='GitHub' },
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
        $priorBottom = $rect.Bottom
    }
    'Author and About entries have clear native names, Tab support, and unobscured sidebar bounds; neither action was activated.'
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
$priorStatus = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_STATUS')
$priorError = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_ERROR')
$priorLongAdapter = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_LONG_ADAPTER')
$priorUnbrokenAdapter = [Environment]::GetEnvironmentVariable('CAMPUS_PREVIEW_UNBROKEN_ADAPTER')
$errorPath = Join-Path $captureDir 'error.txt'
$priorDpiContext = [GuardianPreviewNative]::SetThreadDpiAwarenessContext([IntPtr](-4))
try {
    $env:CAMPUS_UI_PREVIEW_DPI = [string]$Dpi
    $env:CAMPUS_UI_PREVIEW_WORK_WIDTH = [string]$WorkAreaWidth
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
    if ($DiagnoseStatus) {
        Get-PreviewStatusDiagnostics $previewWindow | Write-Output
    }
} finally {
    [Environment]::SetEnvironmentVariable('CAMPUS_UI_PREVIEW_DPI', $priorDpi)
    [Environment]::SetEnvironmentVariable('CAMPUS_UI_PREVIEW_WORK_WIDTH', $priorWidth)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_STATUS', $priorStatus)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_ERROR', $priorError)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_LONG_ADAPTER', $priorLongAdapter)
    [Environment]::SetEnvironmentVariable('CAMPUS_PREVIEW_UNBROKEN_ADAPTER', $priorUnbrokenAdapter)
    if ($previewProcess -and -not $previewProcess.HasExited) {
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
