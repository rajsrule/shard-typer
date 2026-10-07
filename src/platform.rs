use crate::{
    engine::{Command, InputAdapter},
    settings::NewlineMode,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

#[cfg(windows)]
#[path = "backdrop.rs"]
mod backdrop;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeySpec {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub windows: bool,
    pub function: u32,
}
impl Default for HotkeySpec {
    fn default() -> Self {
        Self {
            control: true,
            alt: true,
            shift: false,
            windows: false,
            function: 8,
        }
    }
}
impl HotkeySpec {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=24).contains(&self.function) {
            return Err("Choose a function key from F1 to F24.".into());
        }
        if self.function == 12 {
            return Err(
                "Windows reserves F12 for debugging. Choose a different function key.".into(),
            );
        }
        Ok(())
    }
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.control {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.windows {
            parts.push("Win".into());
        }
        parts.push(format!("F{}", self.function));
        parts.join(" + ")
    }
}

pub struct PlatformInput;
pub enum HotkeyEvent {
    Toggle,
    Stopped,
    Registered(Result<HotkeySpec, String>),
    EscapeError(String),
}
pub enum HotkeyCommand {
    Configure(HotkeySpec),
    Escape(bool),
    Shutdown,
}
pub struct HotkeyService {
    pub tx: mpsc::Sender<HotkeyCommand>,
    pub rx: mpsc::Receiver<HotkeyEvent>,
    handle: Option<thread::JoinHandle<()>>,
}
impl HotkeyService {
    pub fn spawn(
        spec: HotkeySpec,
        engine: mpsc::Sender<Command>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let handle =
            thread::spawn(move || native::hotkey_loop(spec, engine, wake, commands, events));
        Self {
            tx,
            rx,
            handle: Some(handle),
        }
    }
}
impl Drop for HotkeyService {
    fn drop(&mut self) {
        let _ = self.tx.send(HotkeyCommand::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(windows)]
mod native {
    use super::backdrop::BackdropWindow;
    use super::*;
    use crate::silhouette::ContinuousRect;
    use std::{ffi::c_void, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Dwm::*,
        System::{DataExchange::*, Memory::*, Threading::*},
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::*,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    };

    pub fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    impl InputAdapter for PlatformInput {
        fn foreground(&self) -> Option<u64> {
            let hwnd = unsafe { GetForegroundWindow() };
            if hwnd.is_null() || unsafe { GetWindowThreadProcessId(hwnd, null_mut()) } == 0 {
                None
            } else {
                Some(hwnd as u64)
            }
        }
        fn is_own_window(&self, target: u64) -> bool {
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(target as HWND, &mut pid);
            }
            pid == std::process::id()
        }
        fn modifiers_down(&self) -> bool {
            [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
                .into_iter()
                .chain(VK_F1..=VK_F24)
                .any(|key| unsafe { GetAsyncKeyState(key as i32) < 0 })
        }
        fn send(&mut self, text: &str, newline: NewlineMode) -> Result<(), String> {
            let mut events = Vec::new();
            if text == "\n" {
                if newline == NewlineMode::ShiftEnter {
                    events.push(key(VK_SHIFT, 0, 0));
                }
                events.push(key(VK_RETURN, 0, 0));
                events.push(key(VK_RETURN, 0, KEYEVENTF_KEYUP));
                if newline == NewlineMode::ShiftEnter {
                    events.push(key(VK_SHIFT, 0, KEYEVENTF_KEYUP));
                }
            } else {
                for unit in text.encode_utf16() {
                    events.push(key(0, unit, KEYEVENTF_UNICODE));
                    events.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
            }
            let sent = unsafe {
                SendInput(
                    events.len() as u32,
                    events.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                )
            };
            if sent != events.len() as u32 {
                // Best effort release if a partial soft-newline packet left Shift down.
                if text == "\n" && newline == NewlineMode::ShiftEnter {
                    let release = key(VK_SHIFT, 0, KEYEVENTF_KEYUP);
                    unsafe {
                        SendInput(1, &release, std::mem::size_of::<INPUT>() as i32);
                    }
                }
                return Err(format!(
                    "Windows accepted {sent}/{} input events. Typing stopped to avoid duplicates. The target may block input or be running as administrator.",
                    events.len()
                ));
            }
            Ok(())
        }
    }
    fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
    fn register(spec: &HotkeySpec) -> Result<(), String> {
        spec.validate()?;
        let mut flags = MOD_NOREPEAT;
        if spec.control {
            flags |= MOD_CONTROL;
        }
        if spec.alt {
            flags |= MOD_ALT;
        }
        if spec.shift {
            flags |= MOD_SHIFT;
        }
        if spec.windows {
            flags |= MOD_WIN;
        }
        if unsafe { RegisterHotKey(null_mut(), 1, flags, VK_F1 as u32 + spec.function - 1) } == 0 {
            Err(format!(
                "{} is unavailable (Windows error {}). Choose another hotkey. Cursor mode still works.",
                spec.label(),
                unsafe { GetLastError() }
            ))
        } else {
            Ok(())
        }
    }
    pub fn hotkey_loop(
        mut spec: HotkeySpec,
        engine: mpsc::Sender<Command>,
        wake: Arc<dyn Fn() + Send + Sync>,
        commands: mpsc::Receiver<HotkeyCommand>,
        events: mpsc::Sender<HotkeyEvent>,
    ) {
        let first = register(&spec).map(|_| spec.clone());
        let mut registered = first.is_ok();
        let _ = events.send(HotkeyEvent::Registered(first));
        wake();
        let mut escape = false;
        loop {
            match commands.recv_timeout(Duration::from_millis(10)) {
                Ok(HotkeyCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(HotkeyCommand::Configure(next)) => {
                    if registered {
                        unsafe {
                            UnregisterHotKey(null_mut(), 1);
                        }
                    }
                    let result = register(&next);
                    if result.is_ok() {
                        registered = true;
                        spec = next.clone();
                    } else {
                        registered = register(&spec).is_ok();
                    }
                    let _ = events.send(HotkeyEvent::Registered(result.map(|_| next)));
                    wake();
                }
                Ok(HotkeyCommand::Escape(active)) if active != escape => {
                    if active {
                        escape = unsafe {
                            RegisterHotKey(null_mut(), 2, MOD_NOREPEAT, VK_ESCAPE as u32)
                        } != 0;
                        if !escape {
                            let _ = events.send(HotkeyEvent::EscapeError(
                                "Escape is unavailable. Use Stop or the typing hotkey.".into(),
                            ));
                            wake();
                        }
                    } else {
                        unsafe {
                            UnregisterHotKey(null_mut(), 2);
                        }
                        escape = false;
                    }
                }
                _ => {}
            }
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, null_mut(), WM_HOTKEY, WM_HOTKEY, PM_REMOVE) }
                != 0
            {
                if msg.wParam == 1 {
                    let _ = events.send(HotkeyEvent::Toggle);
                }
                if msg.wParam == 2 {
                    let _ = engine.send(Command::Stop);
                    let _ = events.send(HotkeyEvent::Stopped);
                }
                wake();
            }
        }
        if registered {
            unsafe {
                UnregisterHotKey(null_mut(), 1);
            }
        }
        if escape {
            unsafe {
                UnregisterHotKey(null_mut(), 2);
            }
        }
    }
    pub struct SingleInstance(HANDLE);
    impl SingleInstance {
        pub fn acquire() -> Result<Self, String> {
            let handle =
                unsafe { CreateMutexW(null_mut(), 0, wide("Local\\ShardTyper.v1").as_ptr()) };
            if handle.is_null() {
                return Err("Could not create the application instance lock.".into());
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                let hwnd = unsafe { FindWindowW(null_mut(), wide("Shard Typer").as_ptr()) };
                if !hwnd.is_null() {
                    unsafe {
                        ShowWindow(hwnd, SW_RESTORE);
                        SetForegroundWindow(hwnd);
                    }
                }
                unsafe {
                    CloseHandle(handle);
                }
                return Err("Shard Typer is already open.".into());
            }
            Ok(Self(handle))
        }
    }
    impl Drop for SingleInstance {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    const FRAME_SUBCLASS_ID: usize = 0x5348_4152;
    const FRAME_PROPERTY: &[u16] = &[
        83, 104, 97, 114, 100, 84, 121, 112, 101, 114, 46, 67, 117, 115, 116, 111, 109, 70, 114,
        97, 109, 101, 0,
    ];

    struct FrameState {
        blur: bool,
        backdrop: Option<BackdropWindow>,
        blur_error: Option<String>,
        syncing: bool,
        monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR,
        shadow: Option<ShadowWindow>,
        radius: f32,
    }

    unsafe fn sync_layers(hwnd: HWND, reference: usize) {
        unsafe {
            // Native positioning can synchronously dispatch another window
            // message. Read the guard through a raw pointer before borrowing
            // any layer, and never hold a whole-state borrow across the call.
            let state = reference as *mut FrameState;
            if (*state).syncing {
                return;
            }
            (*state).syncing = true;
            let radius = (*state).radius;
            // Insert the shadow first, then the blur immediately below egui.
            // The resulting order is main UI, desktop blur, then shadow.
            if let Some(shadow) = (*state).shadow.as_mut() {
                shadow.sync(hwnd, radius);
            }
            if let Some(backdrop) = (*state).backdrop.as_mut() {
                backdrop.sync(hwnd, radius);
            }
            (*state).syncing = false;
        }
    }

    struct ShadowWindow {
        hwnd: HWND,
        rendered: Option<[i32; 4]>,
    }

    impl ShadowWindow {
        unsafe fn new() -> Option<Self> {
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Shard Typer shadow").as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            (!hwnd.is_null()).then_some(Self {
                hwnd,
                rendered: None,
            })
        }

        fn sync(&mut self, parent: HWND, radius: f32) {
            use windows_sys::Win32::Graphics::Gdi::*;
            unsafe {
                if IsWindowVisible(parent) == 0 || IsIconic(parent) != 0 || IsZoomed(parent) != 0 {
                    ShowWindow(self.hwnd, SW_HIDE);
                    return;
                }
                let mut rect: RECT = std::mem::zeroed();
                if GetWindowRect(parent, &mut rect) == 0 {
                    return;
                }
                let scale = GetDpiForWindow(parent).max(96) as f32 / 96.;
                let margin = (22. * scale).ceil() as i32;
                let body = [rect.right - rect.left, rect.bottom - rect.top];
                if body[0] <= 0 || body[1] <= 0 {
                    return;
                }
                let size = SIZE {
                    cx: body[0] + margin * 2,
                    cy: body[1] + margin * 2,
                };
                let position = POINT {
                    x: rect.left - margin,
                    y: rect.top - margin,
                };
                let key = [
                    body[0],
                    body[1],
                    (scale * 1000.).round() as i32,
                    (radius * 1000.).round() as i32,
                ];
                if self.rendered != Some(key) {
                    let mut bitmap_info: BITMAPINFO = std::mem::zeroed();
                    bitmap_info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                    bitmap_info.bmiHeader.biWidth = size.cx;
                    bitmap_info.bmiHeader.biHeight = -size.cy;
                    bitmap_info.bmiHeader.biPlanes = 1;
                    bitmap_info.bmiHeader.biBitCount = 32;
                    bitmap_info.bmiHeader.biCompression = BI_RGB;
                    let dc = CreateCompatibleDC(null_mut());
                    if dc.is_null() {
                        return;
                    }
                    let mut pixels = null_mut();
                    let bitmap = CreateDIBSection(
                        dc,
                        &bitmap_info,
                        DIB_RGB_COLORS,
                        &mut pixels,
                        null_mut(),
                        0,
                    );
                    if bitmap.is_null() || pixels.is_null() {
                        if !bitmap.is_null() {
                            DeleteObject(bitmap);
                        }
                        DeleteDC(dc);
                        return;
                    }
                    let pixels = std::slice::from_raw_parts_mut(
                        pixels as *mut u32,
                        (size.cx as usize) * (size.cy as usize),
                    );
                    let silhouette = ContinuousRect::new([body[0] as f32, body[1] as f32], radius);
                    let sigma = 7. * scale;
                    for y in 0..size.cy {
                        for x in 0..size.cx {
                            let point = [
                                x as f32 + 0.5 - margin as f32,
                                y as f32 + 0.5 - margin as f32,
                            ];
                            let alpha = shadow_alpha(point, &silhouette, sigma, 3. * scale);
                            // Black is already premultiplied; BGRA alpha is the
                            // high byte. Nothing is painted underneath the body.
                            pixels[y as usize * size.cx as usize + x as usize] =
                                (alpha as u32) << 24;
                        }
                    }
                    let old = SelectObject(dc, bitmap);
                    let source = POINT { x: 0, y: 0 };
                    let blend = BLENDFUNCTION {
                        BlendOp: AC_SRC_OVER as u8,
                        BlendFlags: 0,
                        SourceConstantAlpha: 255,
                        AlphaFormat: AC_SRC_ALPHA as u8,
                    };
                    let uploaded = UpdateLayeredWindow(
                        self.hwnd,
                        null_mut(),
                        &position,
                        &size,
                        dc,
                        &source,
                        0,
                        &blend,
                        ULW_ALPHA,
                    ) != 0;
                    SelectObject(dc, old);
                    DeleteObject(bitmap);
                    DeleteDC(dc);
                    if !uploaded {
                        return;
                    }
                    self.rendered = Some(key);
                }
                // A Win32-owned popup must stay above its owner. This tool HWND
                // is instead lifecycle-owned by FrameState and explicitly kept
                // immediately below the main window, including when pinned.
                SetWindowPos(
                    self.hwnd,
                    parent,
                    position.x,
                    position.y,
                    size.cx,
                    size.cy,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }

    impl Drop for ShadowWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }

    fn shadow_alpha(point: [f32; 2], silhouette: &ContinuousRect, sigma: f32, offset: f32) -> u8 {
        if silhouette.contains(point) {
            return 0;
        }
        let distance = silhouette.signed_distance([point[0], point[1] - offset]);
        let coverage = 0.5 * libm::erfc(f64::from(distance / (sigma * std::f32::consts::SQRT_2)));
        (coverage * 105.).round().clamp(0., 105.) as u8
    }

    fn frameless_style(style: u32) -> u32 {
        // Winit retains WS_CAPTION for snapping and normally hides it with
        // WM_NCCALCSIZE. A transparent DirectComposition surface also exposes
        // DWM's caption painting, so remove the visible frame styles explicitly.
        // WS_THICKFRAME stays intact while expanded, preserving native resizing.
        style & !(WS_CAPTION | WS_BORDER | WS_DLGFRAME)
    }

    fn frameless_ex_style(style: u32) -> u32 {
        // Winit initializes transparent redirection through DWM at creation.
        // NOREDIRECTIONBITMAP cannot be added to an existing HWND: Windows
        // rejects that change with ERROR_INVALID_PARAMETER.
        style & !(WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME)
    }

    fn edge_hit_test(rect: RECT, point: [i32; 2], border: i32) -> u32 {
        let left = point[0] < rect.left + border;
        let right = point[0] >= rect.right - border;
        let top = point[1] < rect.top + border;
        let bottom = point[1] >= rect.bottom - border;
        match (left, right, top, bottom) {
            (true, _, true, _) => HTTOPLEFT,
            (_, true, true, _) => HTTOPRIGHT,
            (true, _, _, true) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, _, _, _) => HTLEFT,
            (_, true, _, _) => HTRIGHT,
            (_, _, true, _) => HTTOP,
            (_, _, _, true) => HTBOTTOM,
            _ => HTCLIENT,
        }
    }

    unsafe extern "system" fn frameless_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        reference: usize,
    ) -> LRESULT {
        // This callback and all users of its state run on the window's UI
        // thread. It owns no references into the egui application.
        unsafe {
            match message {
                WM_STYLECHANGING if lparam != 0 => {
                    let styles = &mut *(lparam as *mut STYLESTRUCT);
                    if wparam as i32 == GWL_STYLE {
                        styles.styleNew = frameless_style(styles.styleNew);
                    } else if wparam as i32 == GWL_EXSTYLE {
                        // Winit rebuilds styles when pinning or collapsing;
                        // prevent a raised native edge beneath the custom one.
                        styles.styleNew = frameless_ex_style(styles.styleNew);
                    }
                    return 0;
                }
                WM_NCCALCSIZE => {
                    if wparam != 0 && IsZoomed(hwnd) != 0 {
                        let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                        let monitor = windows_sys::Win32::Graphics::Gdi::MonitorFromRect(
                            &params.rgrc[0],
                            windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
                        );
                        let mut info: windows_sys::Win32::Graphics::Gdi::MONITORINFO =
                            std::mem::zeroed();
                        info.cbSize = std::mem::size_of_val(&info) as u32;
                        if windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut info)
                            != 0
                        {
                            params.rgrc[0] = info.rcWork;
                        }
                    }
                    // The complete window is client content; there is no hidden
                    // one-pixel nonclient strip or native title bar to paint.
                    return 0;
                }
                WM_NCPAINT => return 0,
                WM_NCACTIVATE => {
                    // The companion hosts its own blur; it has no activation
                    // fallback, so focus can remain in the destination editor.
                    // Retain activation bookkeeping without repainting a native
                    // caption beneath our transparent custom header.
                    return DefSubclassProc(hwnd, message, wparam, -1);
                }
                WM_ERASEBKGND => return 1,
                WM_NCHITTEST => {
                    let mut rect: RECT = std::mem::zeroed();
                    if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_THICKFRAME != 0
                        && IsZoomed(hwnd) == 0
                        && GetWindowRect(hwnd, &mut rect) != 0
                    {
                        // Screen coordinates are signed: the left/top monitor
                        // can have negative coordinates.
                        let point = [lparam as i16 as i32, (lparam >> 16) as i16 as i32];
                        let border =
                            (7. * GetDpiForWindow(hwnd).max(96) as f32 / 96.).round() as i32;
                        let hit = edge_hit_test(rect, point, border);
                        if hit != HTCLIENT {
                            return hit as LRESULT;
                        }
                    }
                    return HTCLIENT as LRESULT;
                }
                WM_THEMECHANGED | WM_DWMCOMPOSITIONCHANGED | WM_DISPLAYCHANGE | WM_DPICHANGED => {
                    clear_system_backdrop(hwnd);
                    sync_layers(hwnd, reference);
                }
                WM_WINDOWPOSCHANGED => {
                    let monitor = windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
                        hwnd,
                        windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
                    );
                    let state = reference as *mut FrameState;
                    if (*state).monitor != monitor {
                        (*state).monitor = monitor;
                        // DWM may deliver window messages synchronously. Do not
                        // keep a mutable state reference across that callback.
                        clear_system_backdrop(hwnd);
                    }
                    sync_layers(hwnd, reference);
                }
                WM_WINDOWPOSCHANGING if lparam != 0 => {
                    let position = &*(lparam as *const WINDOWPOS);
                    // Hide before the main HWND moves, so the companion cannot
                    // become an exposed input surface during a resize or drag.
                    if position.flags & (SWP_NOMOVE | SWP_NOSIZE) != (SWP_NOMOVE | SWP_NOSIZE) {
                        let state = reference as *mut FrameState;
                        if !(*state).syncing
                            && let Some(backdrop) = (*state).backdrop.as_ref()
                        {
                            backdrop.hide();
                        }
                    }
                }
                WM_SHOWWINDOW | WM_SIZE => {
                    sync_layers(hwnd, reference);
                }
                WM_NCDESTROY => {
                    RemoveWindowSubclass(hwnd, Some(frameless_proc), subclass_id);
                    RemovePropW(hwnd, FRAME_PROPERTY.as_ptr());
                    drop(Box::from_raw(reference as *mut FrameState));
                }
                _ => {}
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
    }

    /// Install the custom frame and update the desktop blur on the UI thread.
    /// This never activates or moves the destination window.
    pub fn apply_glass(hwnd: isize, blur: bool) -> bool {
        let hwnd = hwnd as HWND;
        unsafe {
            if GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId() {
                return false;
            }
            // Keep private HWND state in a documented property. In contrast,
            // importing GetWindowSubclass by name requires common-controls v6
            // activation, which test/embedding executables may not provide.
            let mut reference = GetPropW(hwnd, FRAME_PROPERTY.as_ptr()) as usize;
            if reference != 0 {
                let state = &mut *(reference as *mut FrameState);
                if state.blur != blur {
                    state.blur = blur;
                    state.backdrop = None;
                    state.blur_error = None;
                }
            } else {
                let state = Box::new(FrameState {
                    blur,
                    backdrop: None,
                    blur_error: None,
                    syncing: false,
                    monitor: windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
                        hwnd,
                        windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
                    ),
                    shadow: ShadowWindow::new(),
                    radius: 24. * GetDpiForWindow(hwnd).max(96) as f32 / 96.,
                });
                reference = Box::into_raw(state) as usize;
                if SetWindowSubclass(hwnd, Some(frameless_proc), FRAME_SUBCLASS_ID, reference) == 0
                {
                    drop(Box::from_raw(reference as *mut FrameState));
                    return false;
                }
                if SetPropW(hwnd, FRAME_PROPERTY.as_ptr(), reference as HANDLE) == 0 {
                    RemoveWindowSubclass(hwnd, Some(frameless_proc), FRAME_SUBCLASS_ID);
                    drop(Box::from_raw(reference as *mut FrameState));
                    return false;
                }
            }
            let state = &mut *(reference as *mut FrameState);
            if blur && state.backdrop.is_none() && state.blur_error.is_none() {
                match BackdropWindow::new() {
                    Ok(backdrop) => state.backdrop = Some(backdrop),
                    Err(error) => state.blur_error = Some(error),
                }
            }
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                frameless_style(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32) as isize,
            );
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                frameless_ex_style(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32) as isize,
            );
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            clear_system_backdrop(hwnd);
            sync_layers(hwnd, reference);
            !blur || blur_status(hwnd as isize).is_ok()
        }
    }

    pub fn blur_status(hwnd: isize) -> Result<(), String> {
        unsafe {
            let reference = GetPropW(hwnd as HWND, FRAME_PROPERTY.as_ptr()) as usize;
            if reference == 0 {
                return Err("The desktop blur host is not initialized.".into());
            }
            let state = &*(reference as *const FrameState);
            if let Some(error) = &state.blur_error {
                return Err(error.clone());
            }
            state
                .backdrop
                .as_ref()
                .map_or(Ok(()), BackdropWindow::status)
        }
    }

    fn clear_system_backdrop(hwnd: HWND) {
        let dark = 1_i32;
        let kind = DWMSBT_NONE;
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark as *const _ as *const c_void,
                4,
            );
            // The renderer and window region use the same radius. Native DWM
            // rounding has a different radius and can expose a dark corner rim.
            let corners = DWMWCP_DONOTROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &corners as *const _ as *const c_void,
                4,
            );
            let border = 0xffff_fffe_u32;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR as u32,
                &border as *const _ as *const c_void,
                4,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &kind as *const _ as *const c_void,
                4,
            );
            // The separate host owns the blur. A full native frame extension
            // here would place a second, activation-dependent material below
            // egui and could expose a solid gray surface or corner wedges.
            let margins = windows_sys::Win32::UI::Controls::MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 0,
                cyBottomHeight: 0,
            };
            DwmExtendFrameIntoClientArea(hwnd, &margins);
        }
    }
    pub fn shape_window(hwnd: isize, size: [u32; 2], radius: f32) {
        use windows_sys::Win32::Graphics::Gdi::{
            CreatePolygonRgn, DeleteObject, SetWindowRgn, WINDING,
        };
        unsafe {
            let path = ContinuousRect::new([size[0] as f32, size[1] as f32], radius);
            let points: Vec<POINT> = path
                .points()
                .iter()
                .map(|point| POINT {
                    x: point[0].round() as i32,
                    y: point[1].round() as i32,
                })
                .collect();
            if points.len() < 3 {
                return;
            }
            let region = CreatePolygonRgn(points.as_ptr(), points.len() as i32, WINDING);
            if !region.is_null() && SetWindowRgn(hwnd as HWND, region, 1) == 0 {
                // Windows owns the region only after a successful call.
                DeleteObject(region);
            }
            let reference = GetPropW(hwnd as HWND, FRAME_PROPERTY.as_ptr()) as usize;
            if reference != 0 {
                let state = &mut *(reference as *mut FrameState);
                state.radius = radius;
                sync_layers(hwnd as HWND, reference);
            }
        }
    }

    pub fn paste_text() -> Result<String, String> {
        unsafe {
            if OpenClipboard(null_mut()) == 0 {
                return Err("Clipboard is busy. Try Paste again.".into());
            }
            struct ClipboardGuard;
            impl Drop for ClipboardGuard {
                fn drop(&mut self) {
                    unsafe {
                        CloseClipboard();
                    }
                }
            }
            let _guard = ClipboardGuard;
            let handle = GetClipboardData(13);
            if handle.is_null() {
                return Err("The clipboard has no plain text.".into());
            }
            let pointer = GlobalLock(handle) as *const u16;
            if pointer.is_null() {
                return Err("Could not read clipboard text.".into());
            }
            let len = GlobalSize(handle) / 2;
            let units = std::slice::from_raw_parts(pointer, len);
            let end = units.iter().position(|c| *c == 0).unwrap_or(len);
            let result = String::from_utf16(&units[..end])
                .map_err(|_| "Clipboard text has invalid Unicode.".into());
            GlobalUnlock(handle);
            result
        }
    }
    pub fn visible_position(position: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        let mut rect = RECT {
            left: position[0] as i32,
            top: position[1] as i32,
            right: (position[0] + size[0]) as i32,
            bottom: (position[1] + size[1]) as i32,
        };
        unsafe {
            let monitor = windows_sys::Win32::Graphics::Gdi::MonitorFromRect(
                &rect,
                windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
            );
            let mut info: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of_val(&info) as u32;
            if windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut info) != 0 {
                let w = (rect.right - rect.left).min(info.rcWork.right - info.rcWork.left);
                let h = (rect.bottom - rect.top).min(info.rcWork.bottom - info.rcWork.top);
                rect.left = rect.left.clamp(info.rcWork.left, info.rcWork.right - w);
                rect.top = rect.top.clamp(info.rcWork.top, info.rcWork.bottom - h);
            }
        }
        [rect.left as f32, rect.top as f32]
    }
    #[cfg(test)]
    mod frame_tests {
        use super::*;

        #[test]
        fn desktop_blur_uses_a_real_effect_and_survives_deactivation() {
            unsafe {
                let hwnd = CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                    wide("STATIC").as_ptr(),
                    wide("Shard blur diagnostic").as_ptr(),
                    WS_POPUP | WS_THICKFRAME,
                    -1800,
                    -1800,
                    440,
                    640,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                );
                assert!(!hwnd.is_null());
                struct WindowGuard(HWND);
                impl Drop for WindowGuard {
                    fn drop(&mut self) {
                        unsafe {
                            DestroyWindow(self.0);
                        }
                    }
                }
                let guard = WindowGuard(hwnd);
                let foreground = GetForegroundWindow();
                let enabled = 1i32;
                let supported = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_HOSTBACKDROPBRUSH as u32,
                    &enabled as *const _ as *const c_void,
                    4,
                ) >= 0;
                let initialized = apply_glass(hwnd as isize, true);
                if !supported {
                    assert!(!initialized);
                    assert!(
                        blur_status(hwnd as isize)
                            .unwrap_err()
                            .contains("Windows 11")
                    );
                    assert_eq!(GetForegroundWindow(), foreground);
                    return;
                }
                assert!(initialized, "{}", blur_status(hwnd as isize).unwrap_err());
                let reference = GetPropW(hwnd, FRAME_PROPERTY.as_ptr()) as usize;
                let state = &*(reference as *const FrameState);
                let backdrop = state.backdrop.as_ref().expect("real blur host");
                let blur_hwnd = backdrop.hwnd;
                let shadow_hwnd = state.shadow.as_ref().unwrap().hwnd;
                assert_eq!(GetWindow(blur_hwnd, GW_OWNER), null_mut());
                let styles = GetWindowLongPtrW(blur_hwnd, GWL_EXSTYLE) as u32;
                assert_eq!(
                    styles & (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP),
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP
                );
                assert_eq!(IsWindowVisible(blur_hwnd), 0);
                assert_eq!(
                    SendMessageW(blur_hwnd, WM_NCHITTEST, 0, 0),
                    HTTRANSPARENT as LRESULT
                );

                // Off-screen scratch windows exercise real DWM/Composition
                // without exposing a panel, taking focus or typing any text.
                shape_window(hwnd as isize, [440, 640], 24.);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                sync_layers(hwnd, reference);
                assert_ne!(IsWindowVisible(blur_hwnd), 0);
                (&*(reference as *const FrameState))
                    .backdrop
                    .as_ref()
                    .unwrap()
                    .validate_composition()
                    .expect("host brush and Gaussian effect attached");
                assert_eq!(GetWindow(hwnd, GW_HWNDNEXT), blur_hwnd);
                assert_eq!(GetWindow(blur_hwnd, GW_HWNDNEXT), shadow_hwnd);
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                sync_layers(hwnd, reference);
                assert_ne!(
                    GetWindowLongPtrW(blur_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                    0
                );
                assert_eq!(GetWindow(hwnd, GW_HWNDNEXT), blur_hwnd);
                SetWindowPos(
                    hwnd,
                    HWND_NOTOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                sync_layers(hwnd, reference);
                assert_eq!(
                    GetWindowLongPtrW(blur_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                    0
                );
                let mut parent_rect: RECT = std::mem::zeroed();
                let mut blur_rect: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut parent_rect);
                GetWindowRect(blur_hwnd, &mut blur_rect);
                assert_eq!(
                    [
                        parent_rect.left,
                        parent_rect.top,
                        parent_rect.right,
                        parent_rect.bottom
                    ],
                    [
                        blur_rect.left,
                        blur_rect.top,
                        blur_rect.right,
                        blur_rect.bottom
                    ]
                );
                use windows_sys::Win32::Graphics::Gdi::{
                    CreateRectRgn, DeleteObject, EqualRgn, GetWindowRgn,
                };
                let parent_region = CreateRectRgn(0, 0, 0, 0);
                let blur_region = CreateRectRgn(0, 0, 0, 0);
                assert_ne!(GetWindowRgn(hwnd, parent_region), 0);
                assert_ne!(GetWindowRgn(blur_hwnd, blur_region), 0);
                assert_ne!(EqualRgn(parent_region, blur_region), 0);
                DeleteObject(parent_region);
                DeleteObject(blur_region);
                SendMessageW(hwnd, WM_NCACTIVATE, 0, 0);
                assert_ne!(IsWindowVisible(blur_hwnd), 0);
                blur_status(hwnd as isize).expect("blur stays attached while inactive");
                ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                sync_layers(hwnd, reference);
                assert_ne!(IsIconic(hwnd), 0);
                assert_eq!(IsWindowVisible(blur_hwnd), 0);
                assert_eq!(GetForegroundWindow(), foreground);
                drop(guard);
                assert_eq!(IsWindow(blur_hwnd), 0);
                assert_eq!(IsWindow(shadow_hwnd), 0);
                assert_eq!(GetForegroundWindow(), foreground);
            }
        }

        #[test]
        fn resize_hit_test_handles_negative_monitor_coordinates() {
            let rect = RECT {
                left: -500,
                top: -300,
                right: -60,
                bottom: 340,
            };
            assert_eq!(edge_hit_test(rect, [-499, -299], 7), HTTOPLEFT);
            assert_eq!(edge_hit_test(rect, [-61, -100], 7), HTRIGHT);
            assert_eq!(edge_hit_test(rect, [-200, 339], 7), HTBOTTOM);
            assert_eq!(edge_hit_test(rect, [-200, -100], 7), HTCLIENT);
        }

        #[test]
        fn shadow_is_clear_under_body_and_fades_smoothly_outward() {
            let shape = ContinuousRect::new([440., 640.], 18.);
            assert_eq!(shadow_alpha([220., 320.], &shape, 7., 3.), 0);
            let near = shadow_alpha([441., 320.], &shape, 7., 3.);
            let middle = shadow_alpha([447., 320.], &shape, 7., 3.);
            let far = shadow_alpha([461., 320.], &shape, 7., 3.);
            assert!(near > middle && middle > far);
            assert!(far <= 1);
        }

        #[test]
        fn custom_frame_survives_runtime_style_changes_without_activating() {
            unsafe {
                // A hidden scratch window owned by this test thread. No target
                // editor, global hotkey, or user's running app is touched.
                let hwnd = CreateWindowExW(
                    WS_EX_WINDOWEDGE,
                    wide("STATIC").as_ptr(),
                    wide("Shard frame test").as_ptr(),
                    WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
                    -500,
                    -300,
                    440,
                    640,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                );
                assert!(!hwnd.is_null(), "scratch window creation failed");
                struct WindowGuard(HWND);
                impl Drop for WindowGuard {
                    fn drop(&mut self) {
                        unsafe {
                            DestroyWindow(self.0);
                        }
                    }
                }
                let _guard = WindowGuard(hwnd);
                let foreground = GetForegroundWindow();
                let _ = apply_glass(hwnd as isize, false);
                let reference = GetPropW(hwnd, FRAME_PROPERTY.as_ptr()) as usize;
                assert_ne!(reference, 0);
                let shadow = (&*(reference as *const FrameState))
                    .shadow
                    .as_ref()
                    .expect("shadow window")
                    .hwnd;
                let shadow_style = GetWindowLongPtrW(shadow, GWL_EXSTYLE) as u32;
                assert_eq!(
                    shadow_style
                        & (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW),
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
                );
                assert_eq!(IsWindowVisible(shadow), 0);
                assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CAPTION, 0);
                assert_ne!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_THICKFRAME, 0);

                // Emulate winit rebuilding native styles after pin/collapse.
                SetWindowLongPtrW(
                    hwnd,
                    GWL_STYLE,
                    (WS_CAPTION | WS_BORDER | WS_THICKFRAME) as isize,
                );
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, WS_EX_WINDOWEDGE as isize);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                assert_eq!(
                    GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & (WS_CAPTION | WS_BORDER),
                    0
                );
                // Windows may synthesize WS_EX_WINDOWEDGE from THICKFRAME;
                // NCCALCSIZE/NCPAINT suppression, rather than that derived bit,
                // determines whether any native border occupies client space.

                let mut client: RECT = std::mem::zeroed();
                let mut outer: RECT = std::mem::zeroed();
                assert_ne!(GetClientRect(hwnd, &mut client), 0);
                assert_ne!(GetWindowRect(hwnd, &mut outer), 0);
                assert_eq!(client.right, outer.right - outer.left);
                assert_eq!(client.bottom, outer.bottom - outer.top);
                assert_eq!(GetForegroundWindow(), foreground);

                // The native region follows the continuous path, including its
                // fuller corners, instead of a second circular silhouette.
                use windows_sys::Win32::Graphics::Gdi::{
                    CreateRectRgn, DeleteObject, GetWindowRgn, PtInRegion,
                };
                shape_window(
                    hwnd as isize,
                    [client.right as u32, client.bottom as u32],
                    24.,
                );
                let region = CreateRectRgn(0, 0, 0, 0);
                assert!(!region.is_null());
                assert_ne!(GetWindowRgn(hwnd, region), 0);
                assert_eq!(PtInRegion(region, 0, 0), 0);
                assert_ne!(PtInRegion(region, 4, 4), 0);
                assert_ne!(PtInRegion(region, client.right / 2, 0), 0);
                assert_ne!(PtInRegion(region, 0, client.bottom / 2), 0);
                assert_eq!(PtInRegion(region, client.right - 1, client.bottom - 1), 0);
                DeleteObject(region);

                let x = outer.left + 1;
                let y = outer.top + 50;
                let point = (x as u16 as usize | ((y as u16 as usize) << 16)) as LPARAM;
                assert_eq!(
                    SendMessageW(hwnd, WM_NCHITTEST, 0, point),
                    HTLEFT as LRESULT
                );
                SetWindowLongPtrW(hwnd, GWL_STYLE, WS_CAPTION as isize);
                assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CAPTION, 0);
                assert_eq!(
                    SendMessageW(hwnd, WM_NCHITTEST, 0, point),
                    HTCLIENT as LRESULT
                );
                drop(_guard);
                assert_eq!(IsWindow(shadow), 0);
                assert_eq!(GetForegroundWindow(), foreground);
            }
        }
    }
}

#[cfg(windows)]
pub use native::{
    SingleInstance, apply_glass, blur_status, paste_text, shape_window, visible_position,
};
#[cfg(not(windows))]
mod native {
    use super::*;
    impl InputAdapter for PlatformInput {
        fn foreground(&self) -> Option<u64> {
            None
        }
        fn is_own_window(&self, _: u64) -> bool {
            true
        }
        fn modifiers_down(&self) -> bool {
            false
        }
        fn send(&mut self, _: &str, _: NewlineMode) -> Result<(), String> {
            Err("Typing is supported on Windows only.".into())
        }
    }
    pub fn hotkey_loop(
        _: HotkeySpec,
        _: mpsc::Sender<Command>,
        _: Arc<dyn Fn() + Send + Sync>,
        commands: mpsc::Receiver<HotkeyCommand>,
        _: mpsc::Sender<HotkeyEvent>,
    ) {
        while !matches!(commands.recv(), Ok(HotkeyCommand::Shutdown) | Err(_)) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hotkeys_are_non_text_and_reserved_keys_are_rejected() {
        assert_eq!(HotkeySpec::default().label(), "Ctrl + Alt + F8");
        assert!(
            HotkeySpec {
                function: 12,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            HotkeySpec {
                function: 25,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
