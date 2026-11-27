fn normalize_notes(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let normalized: String = normalized
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect();
    let mut lines = Vec::new();
    for line in normalized.trim().lines() {
        let mut line = line.trim();
        let heading = line.bytes().take_while(|byte| *byte == b'#').count();
        if (1..=6).contains(&heading) && line.as_bytes().get(heading) == Some(&b' ') {
            line = line[heading..].trim_start();
        }
        let line = if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            format!("• {}", item.trim_start())
        } else {
            line.to_owned()
        };
        if !line.is_empty() || lines.last().is_some_and(|line: &String| !line.is_empty()) {
            lines.push(line);
        }
    }
    let result = lines.join("\r\n");
    if result.chars().count() <= 16_384 {
        result
    } else {
        let mut shortened: String = result.chars().take(16_383).collect();
        shortened.push('…');
        shortened
    }
}

#[cfg(windows)]
pub(super) use win::{close, dispatch_dialog, open};

#[cfg(windows)]
mod win {
    use super::super::{UpdateInfo, UpdateState, APPLICATION_VERSION};
    use super::normalize_notes;
    use std::{
        cell::{Cell, RefCell},
        ffi::c_void,
        ptr::null,
    };

    type Handle = isize;
    type Snapshot = fn() -> (UpdateState, UpdateInfo);
    const CLASS: &str = "CampusGuardianUpdateNotice";
    const STYLE: u32 = 0x00cf0000;
    const HEADER: i32 = 600;
    const CURRENT: i32 = 601;
    const LABELS: [i32; 4] = [610, 611, 612, 613];
    const VALUES: [i32; 4] = [620, 621, 622, 623];
    const NOTES_LABEL: i32 = 630;
    const NOTES: i32 = 631;
    const CHECK: i32 = 1;
    const CLOSE: i32 = 2;
    const WM_SETFONT: u32 = 0x30;
    const DM_GETDEFID: u32 = 0x400;
    const DC_HASDEFID: isize = 0x534b;
    thread_local! { static WINDOW: Cell<Handle> = const { Cell::new(0) }; }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct MonitorInfo {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
    }
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: Point,
        private: u32,
    }
    #[repr(C)]
    struct MinMaxInfo {
        reserved: Point,
        max_size: Point,
        max_position: Point,
        min_track: Point,
        max_track: Point,
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
    struct CreateStruct {
        params: *mut c_void,
        instance: Handle,
        menu: Handle,
        parent: Handle,
        height: i32,
        width: i32,
        y: i32,
        x: i32,
        style: i32,
        name: *const u16,
        class: *const u16,
        ex_style: u32,
    }
    struct Notice {
        dpi: Cell<u32>,
        font: Cell<Handle>,
        title_font: Cell<Handle>,
        snapshot: Snapshot,
        request: fn(),
        last: RefCell<Option<(UpdateState, UpdateInfo)>>,
        body: RefCell<String>,
    }
    impl Drop for Notice {
        fn drop(&mut self) {
            unsafe {
                for font in [self.font.get(), self.title_font.get()] {
                    if font != 0 {
                        DeleteObject(font);
                    }
                }
            }
        }
    }
    struct CreatePayload {
        notice: Option<Box<Notice>>,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(class: *const WndClass) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            owner: Handle,
            menu: Handle,
            instance: Handle,
            params: *const c_void,
        ) -> Handle;
        fn DefWindowProcW(hwnd: Handle, message: u32, w: usize, l: isize) -> isize;
        fn GetWindowLongPtrW(hwnd: Handle, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: Handle, index: i32, value: isize) -> isize;
        fn SendMessageW(hwnd: Handle, message: u32, w: usize, l: isize) -> isize;
        fn SetWindowTextW(hwnd: Handle, value: *const u16) -> i32;
        fn GetDlgItem(hwnd: Handle, id: i32) -> Handle;
        fn ShowWindow(hwnd: Handle, command: i32) -> i32;
        fn SetForegroundWindow(hwnd: Handle) -> i32;
        fn SetFocus(hwnd: Handle) -> Handle;
        fn GetFocus() -> Handle;
        fn GetNextDlgTabItem(hwnd: Handle, control: Handle, previous: i32) -> Handle;
        fn DestroyWindow(hwnd: Handle) -> i32;
        fn IsWindow(hwnd: Handle) -> i32;
        fn IsWindowEnabled(hwnd: Handle) -> i32;
        fn IsChild(parent: Handle, child: Handle) -> i32;
        fn IsDialogMessageW(hwnd: Handle, message: *mut Msg) -> i32;
        fn EnableWindow(hwnd: Handle, enabled: i32) -> i32;
        fn MoveWindow(hwnd: Handle, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
        fn SetWindowPos(
            hwnd: Handle,
            after: Handle,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            flags: u32,
        ) -> i32;
        fn GetClientRect(hwnd: Handle, rect: *mut Rect) -> i32;
        fn GetWindowRect(hwnd: Handle, rect: *mut Rect) -> i32;
        fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex: u32) -> i32;
        fn SetTimer(hwnd: Handle, id: usize, delay: u32, callback: *const c_void) -> usize;
        fn KillTimer(hwnd: Handle, id: usize) -> i32;
        fn SystemParametersInfoW(action: u32, param: u32, data: *mut c_void, flags: u32) -> i32;
        fn MonitorFromWindow(hwnd: Handle, flags: u32) -> Handle;
        fn GetMonitorInfoW(monitor: Handle, info: *mut MonitorInfo) -> i32;
        fn GetSysColor(index: i32) -> u32;
        fn GetSysColorBrush(index: i32) -> Handle;
        fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
        fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
        fn GetDC(hwnd: Handle) -> Handle;
        fn ReleaseDC(hwnd: Handle, dc: Handle) -> i32;
        fn DrawTextW(dc: Handle, text: *const u16, count: i32, rect: *mut Rect, format: u32)
            -> i32;
        fn InvalidateRect(hwnd: Handle, rect: *const Rect, erase: i32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }
    #[link(name = "gdi32")]
    extern "system" {
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
            output: u32,
            clip: u32,
            quality: u32,
            pitch: u32,
            face: *const u16,
        ) -> Handle;
        fn DeleteObject(object: Handle) -> i32;
        fn SelectObject(dc: Handle, object: Handle) -> Handle;
        fn SetTextColor(dc: Handle, color: u32) -> u32;
        fn SetBkColor(dc: Handle, color: u32) -> u32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn scale(value: i32, dpi: u32) -> i32 {
        ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32
    }
    unsafe fn make_font(dpi: u32, size: i32, weight: i32) -> Handle {
        CreateFontW(
            -scale(size, dpi),
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
    unsafe fn work_area(hwnd: Handle) -> Rect {
        let mut info = MonitorInfo {
            size: std::mem::size_of::<MonitorInfo>() as u32,
            monitor: Rect::default(),
            work: Rect::default(),
            flags: 0,
        };
        let work = if GetMonitorInfoW(MonitorFromWindow(hwnd, 2), &mut info) != 0 {
            info.work
        } else {
            let mut fallback = Rect {
                right: 1024,
                bottom: 768,
                ..Rect::default()
            };
            SystemParametersInfoW(0x30, 0, (&mut fallback as *mut Rect).cast(), 0);
            fallback
        };
        #[cfg(debug_assertions)]
        let work = {
            let mut work = work;
            let dimension = |key| {
                std::env::var(key)
                    .ok()
                    .and_then(|value| value.parse::<i32>().ok())
                    .filter(|size| *size > 0)
            };
            if let Some(width) = dimension("CAMPUS_UI_PREVIEW_WORK_WIDTH") {
                work.right = work.left + width;
            }
            if let Some(height) = dimension("CAMPUS_UI_PREVIEW_NOTICE_HEIGHT") {
                work.bottom = work.top + height;
            }
            work
        };
        work
    }

    /// All calls and snapshots stay on the existing UI thread.
    pub(crate) unsafe fn open(owner: Handle, dpi: u32, snapshot: Snapshot, request: fn()) {
        let mut owner_rect = Rect::default();
        GetWindowRect(owner, &mut owner_rect);
        let preview = owner_rect.right < -10000 || owner_rect.bottom < -10000;
        let existing = WINDOW.with(Cell::get);
        if existing != 0 && IsWindow(existing) != 0 {
            ShowWindow(existing, if preview { 4 } else { 9 });
            if !preview {
                SetForegroundWindow(existing);
            }
            return;
        }
        let dpi = dpi.clamp(96, 480);
        let instance = GetModuleHandleW(null());
        let class = wide(CLASS);
        let wc = WndClass {
            style: 0,
            proc: Some(wndproc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: LoadIconW(instance, 1usize as *const u16),
            cursor: LoadCursorW(0, 32512usize as *const u16),
            background: 16,
            menu: null(),
            class: class.as_ptr(),
        };
        // The class remains registered for the lifetime of this process.
        RegisterClassW(&wc);
        let mut payload = CreatePayload {
            notice: Some(Box::new(Notice {
                dpi: Cell::new(dpi),
                font: Cell::new(make_font(dpi, 14, 400)),
                title_font: Cell::new(make_font(dpi, 21, 600)),
                snapshot,
                request,
                last: RefCell::new(None),
                body: RefCell::new(String::new()),
            })),
        };
        let mut frame = Rect {
            right: scale(600, dpi),
            bottom: scale(540, dpi),
            ..Rect::default()
        };
        AdjustWindowRectEx(&mut frame, STYLE, 0, 0x10000);
        let work = work_area(owner);
        let width = (frame.right - frame.left).min(work.right - work.left);
        let height = (frame.bottom - frame.top).min(work.bottom - work.top);
        let (x, y) = if preview {
            (owner_rect.left + 24, owner_rect.top + 24)
        } else {
            (
                (owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2)
                    .clamp(work.left, (work.right - width).max(work.left)),
                (owner_rect.top + 32).clamp(work.top, (work.bottom - height).max(work.top)),
            )
        };
        let hwnd = CreateWindowExW(
            0x10000,
            class.as_ptr(),
            wide("更新与公告").as_ptr(),
            STYLE,
            x,
            y,
            width,
            height,
            owner,
            0,
            instance,
            (&mut payload as *mut CreatePayload).cast(),
        );
        if hwnd != 0 {
            WINDOW.with(|window| window.set(hwnd));
            ShowWindow(hwnd, if preview { 4 } else { 1 });
            if !preview {
                SetForegroundWindow(hwnd);
                SetFocus(GetDlgItem(hwnd, NOTES));
            }
        }
    }

    pub(crate) unsafe fn close() {
        let hwnd = WINDOW.with(Cell::get);
        if hwnd != 0 && IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
    }

    pub(crate) unsafe fn dispatch_dialog(message: *mut c_void) -> bool {
        let hwnd = WINDOW.with(Cell::get);
        let message = message.cast::<Msg>();
        hwnd != 0
            && !message.is_null()
            && ((*message).hwnd == hwnd || IsChild(hwnd, (*message).hwnd) != 0)
            && IsDialogMessageW(hwnd, message) != 0
    }

    unsafe fn child(hwnd: Handle, class: &str, caption: &str, style: u32, id: i32) -> Handle {
        CreateWindowExW(
            if id == NOTES { 0x200 } else { 0 },
            wide(class).as_ptr(),
            wide(caption).as_ptr(),
            0x50000000 | style,
            0,
            0,
            1,
            1,
            hwnd,
            id as Handle,
            GetModuleHandleW(null()),
            null(),
        )
    }
    unsafe fn controls(hwnd: Handle, notice: &Notice) -> bool {
        child(hwnd, "STATIC", "更新与公告", 0x80, HEADER);
        child(
            hwnd,
            "STATIC",
            &format!("当前应用 v{APPLICATION_VERSION} · 自动检查在后台进行"),
            0x4080,
            CURRENT,
        );
        for (id, caption) in
            LABELS
                .into_iter()
                .zip(["运行状态", "发行版本", "更新来源", "下载阶段"])
        {
            child(hwnd, "STATIC", caption, 0x80, id);
        }
        for id in VALUES {
            child(hwnd, "STATIC", "", 0x80, id);
        }
        child(hwnd, "STATIC", "版本说明", 0x80, NOTES_LABEL);
        child(hwnd, "EDIT", "", 0x00210844, NOTES);
        child(hwnd, "BUTTON", "重新检查", 0x10001, CHECK);
        child(hwnd, "BUTTON", "关闭", 0x10000, CLOSE);
        apply_fonts(hwnd, notice);
        SendMessageW(GetDlgItem(hwnd, NOTES), 0xc5, 65536, 0);
        [
            HEADER,
            CURRENT,
            610,
            611,
            612,
            613,
            620,
            621,
            622,
            623,
            NOTES_LABEL,
            NOTES,
            CHECK,
            CLOSE,
        ]
        .into_iter()
        .all(|id| GetDlgItem(hwnd, id) != 0)
    }
    unsafe fn apply_fonts(hwnd: Handle, notice: &Notice) {
        for id in [
            CURRENT,
            610,
            611,
            612,
            613,
            620,
            621,
            622,
            623,
            NOTES_LABEL,
            NOTES,
            CHECK,
            CLOSE,
        ] {
            SendMessageW(
                GetDlgItem(hwnd, id),
                WM_SETFONT,
                notice.font.get() as usize,
                1,
            );
        }
        SendMessageW(
            GetDlgItem(hwnd, HEADER),
            WM_SETFONT,
            notice.title_font.get() as usize,
            1,
        );
    }
    unsafe fn put(hwnd: Handle, id: i32, text: &str) {
        SetWindowTextW(GetDlgItem(hwnd, id), wide(text).as_ptr());
    }
    unsafe fn refresh(hwnd: Handle, notice: &Notice) {
        let (state, info) = (notice.snapshot)();
        let previous = notice.last.borrow();
        if previous
            .as_ref()
            .is_some_and(|(previous_state, previous_info)| {
                *previous_state == state && *previous_info == info
            })
        {
            return;
        }
        let before = previous.as_ref().map(|(_, info)| info);
        if previous.as_ref().map(|(state, _)| *state) != Some(state) {
            put(
                hwnd,
                VALUES[0],
                match state {
                    UpdateState::Idle => "尚未检查",
                    UpdateState::Checking => "正在检查",
                    UpdateState::Current => "已是最新版本",
                    UpdateState::Installing => "正在安装更新",
                    UpdateState::Failed => "更新未完成，可重新检查",
                },
            );
            let check = GetDlgItem(hwnd, CHECK);
            let available = !matches!(state, UpdateState::Checking | UpdateState::Installing);
            if !available && GetFocus() == check {
                SetFocus(GetDlgItem(hwnd, CLOSE));
            }
            EnableWindow(check, i32::from(available));
        }
        if before.map(|old| &old.version) != Some(&info.version) {
            put(
                hwnd,
                VALUES[1],
                if info.version.is_empty() {
                    "尚未获取"
                } else {
                    &info.version
                },
            );
        }
        if before.map(|old| &old.source) != Some(&info.source) {
            put(
                hwnd,
                VALUES[2],
                if info.source.is_empty() {
                    "尚未获取"
                } else {
                    &info.source
                },
            );
        }
        if before.map(|old| &old.progress) != Some(&info.progress) {
            put(
                hwnd,
                VALUES[3],
                if info.progress.is_empty() {
                    "等待检查更新"
                } else {
                    &info.progress
                },
            );
        }
        if before.map(|old| &old.changelog) != Some(&info.changelog) {
            let body = normalize_notes(&info.changelog);
            let body = if body.is_empty() {
                "尚未获取版本说明。检查完成后将在此显示。".into()
            } else {
                body
            };
            if *notice.body.borrow() != body {
                put(hwnd, NOTES, &body);
                *notice.body.borrow_mut() = body;
            }
        }
        let layout_changed = before.map(|old| &old.source) != Some(&info.source);
        drop(previous);
        *notice.last.borrow_mut() = Some((state, info));
        if layout_changed {
            layout(hwnd, notice);
        }
    }
    unsafe fn layout(hwnd: Handle, notice: &Notice) {
        let dpi = notice.dpi.get();
        let s = |value| scale(value, dpi);
        let mut client = Rect::default();
        GetClientRect(hwnd, &mut client);
        let compact = client.bottom < s(440);
        let short = client.bottom < s(360);
        let margin = s(if compact { 18 } else { 24 });
        let width = (client.right - 2 * margin).max(s(120));
        let value_x = margin + s(88);
        let value_width = (width - s(88)).max(s(40));
        let bounds = |id, x, y, width, height: i32| {
            MoveWindow(GetDlgItem(hwnd, id), x, y, width, height.max(1), 1);
        };
        if short {
            let subtitle_offset = s(120);
            bounds(HEADER, margin, s(10), s(112), s(30));
            bounds(
                CURRENT,
                margin + subtitle_offset,
                s(16),
                (width - subtitle_offset).max(1),
                s(23),
            );
        } else {
            bounds(
                HEADER,
                margin,
                s(if compact { 12 } else { 18 }),
                width,
                s(30),
            );
            bounds(
                CURRENT,
                margin,
                s(if compact { 47 } else { 55 }),
                width,
                s(23),
            );
        }
        let source = notice
            .last
            .borrow()
            .as_ref()
            .map(|(_, info)| info.source.clone())
            .unwrap_or_default();
        let mut source_rect = Rect {
            right: value_width,
            ..Rect::default()
        };
        let dc = GetDC(hwnd);
        let old_font = SelectObject(dc, notice.font.get());
        DrawTextW(
            dc,
            wide(&source).as_ptr(),
            -1,
            &mut source_rect,
            0x400 | 0x10 | 0x800,
        );
        SelectObject(dc, old_font);
        ReleaseDC(hwnd, dc);
        let row_height = s(if short {
            21
        } else if compact {
            24
        } else {
            28
        });
        let source_height = (source_rect.bottom + s(6)).max(row_height);
        let mut y = s(if short {
            46
        } else if compact {
            80
        } else {
            94
        });
        for row in 0..4 {
            let height = if row == 2 { source_height } else { row_height };
            bounds(LABELS[row], margin, y, s(80), height);
            bounds(VALUES[row], value_x, y, value_width, height);
            y += height;
        }
        bounds(
            NOTES_LABEL,
            margin,
            y + s(if short { 4 } else { 8 }),
            width,
            s(24),
        );
        let button_y = client.bottom
            - s(if short {
                12 + 34
            } else if compact {
                16 + 34
            } else {
                20 + 34
            });
        let body_y = y + s(if short {
            24
        } else if compact {
            32
        } else {
            36
        });
        bounds(
            NOTES,
            margin,
            body_y,
            width,
            (button_y
                - s(if short {
                    8
                } else if compact {
                    10
                } else {
                    16
                })
                - body_y)
                .max(1),
        );
        bounds(
            CHECK,
            client.right - margin - s(204),
            button_y,
            s(108),
            s(34),
        );
        bounds(CLOSE, client.right - margin - s(84), button_y, s(84), s(34));
        let edit = GetDlgItem(hwnd, NOTES);
        let mut inner = Rect::default();
        GetClientRect(edit, &mut inner);
        inner.left = s(12);
        inner.right -= s(12);
        let vertical_padding = s(if short { 8 } else { 12 });
        inner.top = vertical_padding;
        inner.bottom -= vertical_padding;
        // Keep the last visible line whole as the window is resized.
        let first_line = SendMessageW(edit, 0xce, 0, 0) as usize;
        let line_top = |line| {
            let index = SendMessageW(edit, 0xbb, line, 0);
            (index >= 0).then(|| (SendMessageW(edit, 0xd6, index as usize, 0) >> 16) as i16 as i32)
        };
        if let (Some(first), Some(second)) = (line_top(first_line), line_top(first_line + 1)) {
            let line_height = second - first;
            if line_height > 0 {
                let visible_lines = ((inner.bottom - inner.top) / line_height).max(1);
                inner.bottom = inner.top + visible_lines * line_height;
            }
        }
        SendMessageW(edit, 0xb3, 0, (&inner as *const Rect) as isize);
    }

    unsafe extern "system" fn wndproc(hwnd: Handle, message: u32, w: usize, l: isize) -> isize {
        if message == 0x81 {
            let create = &*(l as *const CreateStruct);
            let payload = &mut *(create.params as *mut CreatePayload);
            let Some(notice) = payload.notice.take() else {
                return 0;
            };
            SetWindowLongPtrW(hwnd, -21, Box::into_raw(notice) as isize);
            return DefWindowProcW(hwnd, message, w, l);
        }
        let pointer = GetWindowLongPtrW(hwnd, -21) as *mut Notice;
        let Some(notice) = pointer.as_ref() else {
            return DefWindowProcW(hwnd, message, w, l);
        };
        match message {
            1 => {
                if !controls(hwnd, notice) {
                    return -1;
                }
                refresh(hwnd, notice);
                SetTimer(hwnd, 1, 500, null());
                0
            }
            5 => {
                layout(hwnd, notice);
                0
            }
            0x113 => {
                if w == 1 {
                    refresh(hwnd, notice);
                }
                0
            }
            0x111 if w >> 16 == 0 => {
                match (w & 0xffff) as i32 {
                    CHECK => {
                        if !matches!(
                            (notice.snapshot)().0,
                            UpdateState::Checking | UpdateState::Installing
                        ) {
                            (notice.request)();
                            refresh(hwnd, notice);
                        }
                    }
                    CLOSE => {
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                0
            }
            0x28 => {
                let target = if l != 0 {
                    w as Handle
                } else {
                    GetNextDlgTabItem(hwnd, GetFocus(), i32::from(w != 0))
                };
                if target != 0 {
                    SetFocus(target);
                }
                0
            }
            DM_GETDEFID => {
                let check = GetDlgItem(hwnd, CHECK);
                let id = if GetFocus() == GetDlgItem(hwnd, CLOSE) || IsWindowEnabled(check) == 0 {
                    CLOSE
                } else {
                    CHECK
                };
                DC_HASDEFID << 16 | id as isize
            }
            0x10 => {
                DestroyWindow(hwnd);
                0
            }
            2 => {
                KillTimer(hwnd, 1);
                0
            }
            0x82 => {
                SetWindowLongPtrW(hwnd, -21, 0);
                WINDOW.with(|window| {
                    if window.get() == hwnd {
                        window.set(0);
                    }
                });
                drop(Box::from_raw(pointer));
                DefWindowProcW(hwnd, message, w, l)
            }
            0x24 if l != 0 => {
                let info = &mut *(l as *mut MinMaxInfo);
                let work = work_area(hwnd);
                info.min_track.x = scale(420, notice.dpi.get()).min(work.right - work.left);
                info.min_track.y = scale(360, notice.dpi.get()).min(work.bottom - work.top);
                0
            }
            0x2e0 if l != 0 => {
                let dpi = (w & 0xffff) as u32;
                notice.dpi.set(dpi.max(96));
                let old = notice.font.replace(make_font(dpi, 14, 400));
                let old_title = notice.title_font.replace(make_font(dpi, 21, 600));
                apply_fonts(hwnd, notice);
                if old != 0 {
                    DeleteObject(old);
                }
                if old_title != 0 {
                    DeleteObject(old_title);
                }
                let rect = &*(l as *const Rect);
                SetWindowPos(
                    hwnd,
                    0,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    0x14,
                );
                layout(hwnd, notice);
                0
            }
            0x133 | 0x138 => {
                let body = l == GetDlgItem(hwnd, NOTES);
                let background = if body { 5 } else { 15 };
                SetTextColor(w as Handle, GetSysColor(if body { 8 } else { 18 }));
                SetBkColor(w as Handle, GetSysColor(background));
                GetSysColorBrush(background)
            }
            0x1a | 0x15 => {
                InvalidateRect(hwnd, null(), 1);
                0
            }
            _ => DefWindowProcW(hwnd, message, w, l),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_notes;

    #[test]
    fn release_notes_share_a_left_edge_without_losing_paragraphs_or_urls() {
        let notes = "  ## 本版更新\n\t- 自动选择线路\n    * 支持分段下载\n\n  作者：Liunian520\n  https://github.com/Liunian520/campus-network-guardian\n";
        assert_eq!(normalize_notes(notes), "本版更新\r\n• 自动选择线路\r\n• 支持分段下载\r\n\r\n作者：Liunian520\r\nhttps://github.com/Liunian520/campus-network-guardian");
    }

    #[test]
    fn existing_windows_newlines_and_numbered_steps_remain_readable() {
        assert_eq!(
            normalize_notes("\r\n1. 下载 ZIP\r\n2. 解压安装\r\n"),
            "1. 下载 ZIP\r\n2. 解压安装"
        );
    }

    #[test]
    fn long_unicode_notes_are_bounded_at_a_character_boundary() {
        let notes = "更新说明".repeat(10000);
        let normalized = normalize_notes(&notes);
        assert!(normalized.chars().count() <= 16_384);
        assert!(normalized.starts_with("更新说明"));
    }
}
