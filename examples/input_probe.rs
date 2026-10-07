//! Opt-in Windows integration probe. It creates and types only into its own
//! disposable editor, and refuses to send if that window loses focus.
use shard_typer::{engine::InputAdapter, platform::PlatformInput, settings::NewlineMode};
use std::{
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{AttachThreadInput, GetCurrentThreadId},
    },
    UI::{
        Input::KeyboardAndMouse::{SetActiveWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn main() -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(null_mut());
        let class = wide("ShardInputProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err("Could not create the integration probe.".into());
        }
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Shard Typer — disposable input test").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            520,
            240,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if window.is_null() {
            return Err("Could not open the disposable editor.".into());
        }
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | ES_MULTILINE as u32 | ES_WANTRETURN as u32,
            10,
            10,
            480,
            160,
            window,
            null_mut(),
            instance,
            null_mut(),
        );
        ShowWindow(window, SW_SHOW);
        let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
        let current_thread = GetCurrentThreadId();
        let attached = foreground_thread != current_thread
            && AttachThreadInput(current_thread, foreground_thread, 1) != 0;
        SetForegroundWindow(window);
        SetActiveWindow(window);
        SetFocus(edit);
        if attached {
            AttachThreadInput(current_thread, foreground_thread, 0);
        }
        pump(100);
        let mut adapter = PlatformInput;
        let mut result = Ok(());
        for (value, newline) in [
            ("Ice ❄ e\u{301} 👩‍💻", NewlineMode::Enter),
            ("\n", NewlineMode::Enter),
            ("Second line", NewlineMode::Enter),
            ("\n", NewlineMode::ShiftEnter),
            ("Soft line", NewlineMode::Enter),
        ] {
            if GetForegroundWindow() != window {
                result = Err("Probe lost focus. No further input was sent.".into());
                break;
            }
            if let Err(e) = adapter.send(value, newline) {
                result = Err(e);
                break;
            }
            pump(100);
        }
        let mut buffer = vec![0_u16; GetWindowTextLengthW(edit) as usize + 1];
        let len = GetWindowTextW(edit, buffer.as_mut_ptr(), buffer.len() as i32);
        let actual = String::from_utf16_lossy(&buffer[..len as usize]);
        DestroyWindow(window);
        UnregisterClassW(class.as_ptr(), instance);
        result?;
        let expected = "Ice ❄ e\u{301} 👩‍💻\r\nSecond line\r\nSoft line";
        if actual != expected {
            return Err(format!("Unexpected text: {actual:?}"));
        }
        println!(
            "PASS: Unicode, combined graphemes, emoji, Enter, and Shift + Enter through real Windows SendInput."
        );
        Ok(())
    }
}
