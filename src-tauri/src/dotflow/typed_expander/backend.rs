//! DotFlow — the Windows Raw Input backend for the typed text expander.
//!
//! Detection uses **Raw Input**, not a `WH_KEYBOARD_LL` hook: a low-level hook sits in the synchronous
//! input path of every keystroke system-wide and Windows silently unhooks it if it's slow, so it lags the
//! whole machine. Raw Input instead delivers key events asynchronously to a hidden **message-only window**
//! via `WM_INPUT`, on our own dedicated thread, touching nothing on the critical path.
//!
//! Flow: a native thread creates a message-only window, registers the keyboard as a raw input sink, and
//! pumps its message loop. Each key-down is classified ([`key_action`]) into a buffer edit / reset / decode.
//! `Decode` keys go through `ToUnicodeEx` (Raw Input gives virtual keys; we reconstruct characters using the
//! tracked modifier/lock state). When the rolling [`ExpanderBuffer`] ends with a known dot-trigger, we
//! **backspace the trigger and paste its expansion** — reusing the tested injection primitives.
//!
//! Self-trigger safety (no feedback loop): the emit runs under a [`crate::clipboard::injection_guard`] and a
//! short trailing "settle" window. While either is active the `WM_INPUT` handler drops all input, so the
//! synthetic backspaces + paste keystrokes the emit generates can never be seen as user typing.

use std::cell::RefCell;
use std::mem::size_of;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::AppHandle;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, ToUnicodeEx, HKL};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, PostThreadMessageW, RegisterClassW, TranslateMessage, HWND_MESSAGE,
    MSG, RI_KEY_BREAK, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_QUIT, WNDCLASSW,
};

use super::{key_action, ExpanderBuffer, KeyAction};

/// Owns the (at most one) running monitor thread. Managed in Tauri state; `start`/`stop` are driven by the
/// `experimental_typed_expander` setting toggle and by boot when the setting is already on.
#[derive(Default)]
pub struct ExpanderController {
    running: Mutex<Option<Running>>,
}

struct Running {
    /// The monitor thread's native id — `PostThreadMessageW(WM_QUIT)` to this wakes its blocked `GetMessageW`
    /// so the loop exits and the thread can be joined.
    thread_id: u32,
    handle: JoinHandle<()>,
}

impl ExpanderController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the keyboard monitor. No-op if already running. Blocks briefly until the thread reports its id
    /// (so a subsequent `stop` can always reach it).
    pub fn start(&self, app: AppHandle) {
        let mut guard = self.running.lock().unwrap();
        if guard.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<u32>();
        let handle = match std::thread::Builder::new()
            .name("dotflow-typed-expander".into())
            .spawn(move || run_monitor(app, tx))
        {
            Ok(h) => h,
            Err(e) => {
                log::error!("Failed to spawn typed-expander thread: {e}");
                return;
            }
        };
        // Wait for the thread to finish setup and hand back its id. If setup fails it sends 0 (or the channel
        // closes), and we treat the monitor as not running.
        let thread_id = rx.recv().unwrap_or(0);
        if thread_id == 0 {
            log::error!("Typed-expander monitor failed to start; not running.");
            let _ = handle.join();
            return;
        }
        log::info!("Typed text expander monitor started (thread {thread_id}).");
        *guard = Some(Running { thread_id, handle });
    }

    /// Stop the monitor thread (if running) and wait for it to exit.
    pub fn stop(&self) {
        let running = self.running.lock().unwrap().take();
        if let Some(Running { thread_id, handle }) = running {
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = handle.join();
            log::info!("Typed text expander monitor stopped.");
        }
    }
}

/// Per-thread monitor state. Lives in a thread-local because the raw `WNDPROC` is a bare C callback that
/// can't capture — it reaches the state through [`STATE`].
struct MonitorState {
    app: AppHandle,
    buffer: ExpanderBuffer,
    /// The 256-entry key-state array `ToUnicodeEx` reads (high bit = pressed; low bit of lock keys = toggled).
    kbd_state: [u8; 256],
    /// While `Some(t)` and `now < t`, all input is dropped — the trailing settle that swallows the async
    /// `WM_INPUT` echoes of our own emit after the injection guard has dropped.
    suppress_until: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<Option<MonitorState>> = const { RefCell::new(None) };
}

/// UTF-16, NUL-terminated window class name.
fn class_name() -> Vec<u16> {
    "DotFlowTypedExpanderWindow\0".encode_utf16().collect()
}

fn run_monitor(app: AppHandle, tx: Sender<u32>) {
    unsafe {
        let hinstance: HINSTANCE = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h.into(),
            Err(e) => {
                log::error!("typed-expander: GetModuleHandleW failed: {e}");
                let _ = tx.send(0);
                return;
            }
        };

        let name = class_name();
        let class_ptr = PCWSTR(name.as_ptr());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_ptr,
            ..Default::default()
        };
        // Registering again after a stop/start returns 0 with ERROR_CLASS_ALREADY_EXISTS — harmless; the
        // class persists for the process, so we don't treat 0 as fatal.
        RegisterClassW(&wc);

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_ptr,
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log::error!("typed-expander: CreateWindowExW failed: {e}");
                let _ = tx.send(0);
                return;
            }
        };

        STATE.with(|s| {
            *s.borrow_mut() = Some(MonitorState {
                app,
                buffer: ExpanderBuffer::new(),
                kbd_state: [0u8; 256],
                suppress_until: None,
            })
        });

        // Register the keyboard (HID usage page 0x01, usage 0x06) as an INPUTSINK so we get WM_INPUT even
        // when our hidden window isn't focused — which it never is.
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x06,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        if RegisterRawInputDevices(&[rid], size_of::<RAWINPUTDEVICE>() as u32).is_err() {
            log::error!("typed-expander: RegisterRawInputDevices failed");
            STATE.with(|s| *s.borrow_mut() = None);
            let _ = tx.send(0);
            return;
        }

        // Setup done — hand our thread id to the controller so it can stop us.
        let _ = tx.send(GetCurrentThreadId());

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        STATE.with(|s| *s.borrow_mut() = None);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().as_mut() {
                handle_raw_input(state, lparam);
            }
        });
        // WM_INPUT still requires DefWindowProc for cleanup — fall through.
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn handle_raw_input(state: &mut MonitorState, lparam: LPARAM) {
    let hri = HRAWINPUT(lparam.0 as *mut _);
    let header_size = size_of::<RAWINPUTHEADER>() as u32;

    // Query size, then fetch.
    let mut size = 0u32;
    GetRawInputData(hri, RID_INPUT, None, &mut size, header_size);
    if size == 0 {
        return;
    }
    let mut buf = vec![0u8; size as usize];
    let got = GetRawInputData(
        hri,
        RID_INPUT,
        Some(buf.as_mut_ptr() as *mut _),
        &mut size,
        header_size,
    );
    if got == u32::MAX || got == 0 {
        return;
    }

    let raw = &*(buf.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }
    let kb = raw.data.keyboard;
    let vkey = kb.VKey;
    // 0xFF is an escaped/overrun sentinel Raw Input uses for some keys; not a real VK.
    if vkey >= 0xFF {
        return;
    }
    let is_break = (kb.Flags & RI_KEY_BREAK as u16) != 0; // key-up

    // Keep the key-state array current on BOTH edges so ToUnicodeEx sees the right modifiers/locks.
    update_kbd_state(&mut state.kbd_state, vkey, is_break);
    if is_break {
        return; // only key-DOWN drives the buffer
    }

    // Drop everything while WE are injecting (dictation or our own emit) or during the trailing settle.
    if crate::clipboard::is_injecting() || state.suppressed_now() {
        return;
    }

    match key_action(vkey) {
        KeyAction::Backspace => state.buffer.backspace(),
        KeyAction::Reset => state.buffer.reset(),
        KeyAction::Ignore => {}
        KeyAction::Decode => {
            if let Some(c) = decode_char(vkey, kb.MakeCode, &state.kbd_state) {
                state.buffer.push(c);
                let table = crate::managers::phrases::wedge_table(&state.app);
                if let Some((n, expansion)) = state.buffer.matched(&table) {
                    state.emit(n, expansion);
                }
            }
        }
    }
}

impl MonitorState {
    fn suppressed_now(&self) -> bool {
        self.suppress_until.is_some_and(|t| Instant::now() < t)
    }

    /// Replace a matched trigger: backspace the `.key` (n chars) then paste the expansion. The whole emit is
    /// wrapped in one injection guard so the monitor ignores every synthetic keystroke it produces, and a
    /// trailing settle covers the async `WM_INPUT` echoes that arrive just after the guard drops.
    fn emit(&mut self, n: usize, expansion: String) {
        self.suppress_until = Some(Instant::now() + Duration::from_millis(500));
        {
            let _guard = crate::clipboard::injection_guard();
            if let Err(e) = crate::clipboard::inject_field_edit(n, "", &self.app) {
                log::warn!("typed-expander: failed to erase trigger: {e}");
            }
            if let Err(e) = crate::clipboard::inject_bulk(&expansion, &self.app) {
                log::warn!("typed-expander: failed to paste expansion: {e}");
            }
        }
        self.buffer.consume(n);
        // Re-arm the settle from the moment the emit actually finished.
        self.suppress_until = Some(Instant::now() + Duration::from_millis(300));
    }
}

/// The keyboard layout of the currently-focused window's thread, so `ToUnicodeEx` maps keys the way the user
/// sees them in the app they're typing into.
unsafe fn active_layout() -> HKL {
    let fg = GetForegroundWindow();
    if fg.0.is_null() {
        return GetKeyboardLayout(0);
    }
    let tid = GetWindowThreadProcessId(fg, None);
    GetKeyboardLayout(tid)
}

/// Decode a `Decode`-class virtual key into a single printable character via `ToUnicodeEx`, honoring the
/// tracked modifier/lock state. Returns `None` for dead keys, control chars, and non-single-char results
/// (accepted v1 simplification — dot-triggers are plain ASCII).
unsafe fn decode_char(vkey: u16, scan: u16, kbd_state: &[u8; 256]) -> Option<char> {
    let mut out = [0u16; 8];
    let n = ToUnicodeEx(
        vkey as u32,
        scan as u32,
        kbd_state,
        &mut out,
        0,
        Some(active_layout()),
    );
    if n != 1 {
        return None;
    }
    let ch = char::from_u32(out[0] as u32)?;
    if ch.is_control() {
        None
    } else {
        Some(ch)
    }
}

/// Update the `ToUnicodeEx` key-state array for one key edge. Sets/clears the pressed high bit, toggles the
/// low bit of lock keys on key-down, and mirrors the L/R modifier variants onto the generic
/// `VK_SHIFT`/`VK_CONTROL`/`VK_MENU` slots (which `ToUnicodeEx` actually consults).
fn update_kbd_state(state: &mut [u8; 256], vkey: u16, is_break: bool) {
    let i = vkey as usize;
    if i >= 256 {
        return;
    }
    if is_break {
        state[i] &= !0x80;
    } else {
        state[i] |= 0x80;
        // Caps / Num / Scroll lock toggle their low bit on each press.
        if matches!(vkey, 0x14 | 0x90 | 0x91) {
            state[i] ^= 0x01;
        }
    }
    // Mirror side-specific modifiers onto the generic VK the decoder reads.
    let generic = match vkey {
        0xA0 | 0xA1 => Some(0x10usize), // L/R Shift → VK_SHIFT
        0xA2 | 0xA3 => Some(0x11),      // L/R Ctrl  → VK_CONTROL
        0xA4 | 0xA5 => Some(0x12),      // L/R Alt   → VK_MENU
        _ => None,
    };
    if let Some(g) = generic {
        if is_break {
            state[g] &= !0x80;
        } else {
            state[g] |= 0x80;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keydown_sets_and_keyup_clears_the_pressed_bit() {
        let mut s = [0u8; 256];
        update_kbd_state(&mut s, b'A' as u16, false);
        assert_eq!(
            s[b'A' as usize] & 0x80,
            0x80,
            "key-down sets the pressed bit"
        );
        update_kbd_state(&mut s, b'A' as u16, true);
        assert_eq!(s[b'A' as usize] & 0x80, 0, "key-up clears the pressed bit");
    }

    #[test]
    fn left_shift_mirrors_onto_the_generic_shift_slot() {
        // ToUnicodeEx reads VK_SHIFT (0x10), but Raw Input may report LSHIFT (0xA0) — the mirror is what makes
        // a shifted character decode correctly.
        let mut s = [0u8; 256];
        update_kbd_state(&mut s, 0xA0, false); // LShift down
        assert_eq!(
            s[0x10] & 0x80,
            0x80,
            "generic VK_SHIFT is set while LShift is held"
        );
        update_kbd_state(&mut s, 0xA0, true); // LShift up
        assert_eq!(
            s[0x10] & 0x80,
            0,
            "generic VK_SHIFT clears when LShift is released"
        );
    }

    #[test]
    fn caps_lock_toggles_its_low_bit_on_each_press() {
        let mut s = [0u8; 256];
        update_kbd_state(&mut s, 0x14, false); // first press
        assert_eq!(s[0x14] & 0x01, 0x01, "caps lock on after first press");
        update_kbd_state(&mut s, 0x14, true); // release does not toggle
        assert_eq!(s[0x14] & 0x01, 0x01, "release leaves the toggle unchanged");
        update_kbd_state(&mut s, 0x14, false); // second press
        assert_eq!(s[0x14] & 0x01, 0, "caps lock off after second press");
    }
}
