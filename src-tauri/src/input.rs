use enigo::{Enigo, Key, Keyboard, Mouse, Settings};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Wrapper for Enigo to store in Tauri's managed state.
/// Enigo is wrapped in a Mutex since it requires mutable access.
pub struct EnigoState(pub Mutex<Enigo>);

impl EnigoState {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Get the current mouse cursor position using the managed Enigo instance.
/// Returns None if the state is not available or if getting the location fails.
pub fn get_cursor_position(app_handle: &AppHandle) -> Option<(i32, i32)> {
    let enigo_state = app_handle.try_state::<EnigoState>()?;
    let enigo = enigo_state.0.lock().ok()?;
    enigo.location().ok()
}

/// Sends a Ctrl+V or Cmd+V paste command using platform-specific virtual key codes.
/// This ensures the paste works regardless of keyboard layout (e.g., Russian, AZERTY, DVORAK).
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_ctrl_v(enigo: &mut Enigo) -> Result<(), String> {
    // Platform-specific key definitions
    #[cfg(target_os = "macos")]
    let (modifier_key, v_key_code) = (Key::Meta, Key::Other(9));
    #[cfg(target_os = "windows")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(target_os = "linux")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Unicode('v'));

    // Press modifier + V
    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    enigo
        .key(v_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click V key: {}", e))?;

    std::thread::sleep(std::time::Duration::from_millis(100));

    enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e))?;

    Ok(())
}

/// Sends a Ctrl+Shift+V paste command.
/// This is commonly used in terminal applications on Linux to paste without formatting.
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_ctrl_shift_v(enigo: &mut Enigo) -> Result<(), String> {
    // Platform-specific key definitions
    #[cfg(target_os = "macos")]
    let (modifier_key, v_key_code) = (Key::Meta, Key::Other(9)); // Cmd+Shift+V on macOS
    #[cfg(target_os = "windows")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(target_os = "linux")]
    let (modifier_key, v_key_code) = (Key::Control, Key::Unicode('v'));

    // Press Ctrl/Cmd + Shift + V
    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    enigo
        .key(Key::Shift, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press Shift key: {}", e))?;
    enigo
        .key(v_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click V key: {}", e))?;

    std::thread::sleep(std::time::Duration::from_millis(100));

    enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Shift key: {}", e))?;
    enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e))?;

    Ok(())
}

/// Sends a Shift+Insert paste command (Windows and Linux only).
/// This is more universal for terminal applications and legacy software.
/// Note: On Wayland, this may not work - callers should check for Wayland and use alternative methods.
pub fn send_paste_shift_insert(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let insert_key_code = Key::Other(0x2D); // VK_INSERT
    #[cfg(not(target_os = "windows"))]
    let insert_key_code = Key::Other(0x76); // XK_Insert (keycode 118 / 0x76, also used as fallback)

    // Press Shift + Insert
    enigo
        .key(Key::Shift, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press Shift key: {}", e))?;
    enigo
        .key(insert_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click Insert key: {}", e))?;

    std::thread::sleep(std::time::Duration::from_millis(100));

    enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Shift key: {}", e))?;

    Ok(())
}

/// Block until the user physically releases every modifier key (Ctrl/Shift/Alt/Win), or `timeout_ms`
/// elapses. The cleanup hotkey calls this before synthesizing Ctrl+C: while the user still holds the trigger
/// combo (e.g. Ctrl+Shift+U), synthetic key-ups don't stick — the OS re-asserts the physically-held keys, so
/// our Ctrl+C keeps landing as Ctrl+Shift+C and copies nothing. Waiting for a real release makes the copy
/// reliable. A tap-and-release takes ~100 ms, so this adds negligible latency.
#[cfg(target_os = "windows")]
pub fn wait_for_modifiers_released(timeout_ms: u64) {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // VK_SHIFT, VK_CONTROL, VK_MENU(alt), VK_LWIN, VK_RWIN
    const MODS: [i32; 5] = [0x10, 0x11, 0x12, 0x5B, 0x5C];
    let start = std::time::Instant::now();
    loop {
        let any_down = MODS
            .iter()
            .any(|&vk| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0);
        if !any_down || start.elapsed().as_millis() as u64 >= timeout_ms {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(not(target_os = "windows"))]
pub fn wait_for_modifiers_released(_timeout_ms: u64) {}

/// Release the modifier keys (Shift/Ctrl/Alt/Meta) at the OS level. Used before the cleanup hotkey
/// synthesizes a Ctrl+C: the user is typically still holding the trigger combo (e.g. Ctrl+Shift+U), and
/// those held modifiers would otherwise turn our synthetic Ctrl+C into Ctrl+Shift+C (which doesn't copy).
/// Sending key-UPs clears the OS modifier state so the following copy is clean.
pub fn release_modifiers(enigo: &mut Enigo) {
    for key in [Key::Shift, Key::Control, Key::Alt, Key::Meta] {
        let _ = enigo.key(key, enigo::Direction::Release);
    }
}

/// Sends a Ctrl+C or Cmd+C copy command using platform-specific virtual key codes (layout-independent).
/// Used by the "clean up selected text" hotkey to grab the current selection onto the clipboard.
pub fn send_copy_ctrl_c(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (modifier_key, c_key_code) = (Key::Meta, Key::Other(8)); // Cmd+C
    #[cfg(target_os = "windows")]
    let (modifier_key, c_key_code) = (Key::Control, Key::Other(0x43)); // VK_C
    #[cfg(target_os = "linux")]
    let (modifier_key, c_key_code) = (Key::Control, Key::Unicode('c'));

    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    enigo
        .key(c_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click C key: {}", e))?;

    std::thread::sleep(std::time::Duration::from_millis(50));

    enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e))?;

    Ok(())
}

/// Pastes text directly using the enigo text method.
/// This tries to use system input methods if possible, otherwise simulates keystrokes one by one.
pub fn paste_text_direct(enigo: &mut Enigo, text: &str) -> Result<(), String> {
    enigo
        .text(text)
        .map_err(|e| format!("Failed to send text directly: {}", e))?;

    Ok(())
}

// --- Win32 foreground/focus helpers (selection-review overlay) -----------------------------------
//
// The review hotkey must capture the field it fired from, then later refocus it to paste the reviewed
// result back. Under the default HandyKeys backend we are NOT the foreground app, so a plain
// `SetForegroundWindow` is denied by the foreground-lock — `force_foreground` performs the standard
// AttachThreadInput dance to reliably steal focus (finding [F2]). HWND is stored/passed as `isize` so
// the state type stays platform-agnostic. On non-Windows these are stubs.

/// The window that currently has foreground focus, as a raw HWND cast to `isize`. `None` if there is no
/// foreground window (or on non-Windows).
#[cfg(target_os = "windows")]
pub fn get_foreground_window() -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd.0 as isize)
    }
}
#[cfg(not(target_os = "windows"))]
pub fn get_foreground_window() -> Option<isize> {
    None
}

/// Attempt to bring `hwnd` to the foreground with a plain `SetForegroundWindow`. Returns whether the OS
/// honored the request (often `false` when we are not already foreground — use `force_foreground`).
#[cfg(target_os = "windows")]
pub fn set_foreground_window(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    unsafe { SetForegroundWindow(HWND(hwnd as *mut _)).as_bool() }
}
#[cfg(not(target_os = "windows"))]
pub fn set_foreground_window(_hwnd: isize) -> bool {
    false
}

/// Reliably bring `target` to the foreground using the AttachThreadInput dance: temporarily attach our
/// thread's input queue to the current foreground window's thread so the OS lets us call
/// `SetForegroundWindow`, then detach. Needed because the HandyKeys backend confers no activation rights
/// [F2]. Returns whether `SetForegroundWindow` reported success (still verify with a foreground poll).
#[cfg(target_os = "windows")]
pub fn force_foreground(target: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
        SetForegroundWindow,
    };
    unsafe {
        let hwnd = HWND(target as *mut _);
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let fg = GetForegroundWindow();
        let cur = GetCurrentThreadId();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let _ = AttachThreadInput(cur, fg_thread, true);
        let _ = BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd).as_bool();
        let _ = AttachThreadInput(cur, fg_thread, false);
        ok
    }
}
#[cfg(not(target_os = "windows"))]
pub fn force_foreground(_target: isize) -> bool {
    false
}

/// Whether `hwnd` still refers to an existing window. Used as a validity guard before refocusing +
/// pasting so a result never lands in a random window if the source app closed.
#[cfg(target_os = "windows")]
pub fn is_window(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}
#[cfg(not(target_os = "windows"))]
pub fn is_window(_hwnd: isize) -> bool {
    true
}
