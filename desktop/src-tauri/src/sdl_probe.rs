//! Live SDL controller probe — the device-identity source for Tier-B controller
//! writers (Mupen64Plus today).
//!
//! Tier-B emulators address controls by *raw SDL-joystick* element index, which
//! differs from the normalized W3C layout Arcadia captures (e.g. Start is
//! normalized button 9 but raw joystick `button(11)` on this project's pad). The
//! only thing guaranteed to agree with what the emulator binds is the *same
//! libSDL2* the emulator uses: we open the pad through SDL's GameController API
//! and read `SDL_GameControllerGetBindFor{Button,Axis}`, which answers exactly the
//! raw element the emulator would record. The result is a
//! [`dreamvault::controller_write::PadMapping`] consumed by the pure encoder.
//!
//! This is a thin hand-rolled FFI over ~10 SDL2 symbols rather than the full
//! `sdl2` crate: we need only the static bind table, never an event loop, and a
//! minimal surface keeps the dependency to the system `libSDL2.so` every targeted
//! emulator already pulls in.

use dreamvault::controller_write::{HatDir, PadMapping, RawElement};

// SDL subsystem flag + hat-mask bits (SDL2 ABI constants).
const SDL_INIT_GAMECONTROLLER: u32 = 0x0000_2000;
const SDL_HAT_UP: i32 = 0x01;
const SDL_HAT_RIGHT: i32 = 0x02;
const SDL_HAT_DOWN: i32 = 0x04;
const SDL_HAT_LEFT: i32 = 0x08;

// SDL_GameControllerBindType.
const BINDTYPE_BUTTON: i32 = 1;
const BINDTYPE_AXIS: i32 = 2;
const BINDTYPE_HAT: i32 = 3;

/// Mirror of `SDL_GameControllerButtonBind` (12-byte, all-int POD returned by
/// value). The union's first word aliases button/axis/hat-index; the second word
/// is the hat mask.
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlGcBind {
    bind_type: i32,
    a: i32,
    b: i32,
}

/// Mirror of `SDL_JoystickGUID` — a 16-byte opaque identifier returned by value.
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlJoystickGuid {
    data: [u8; 16],
}

#[allow(non_snake_case)]
#[link(name = "SDL2")]
extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> i32;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_NumJoysticks() -> i32;
    fn SDL_IsGameController(joystick_index: i32) -> i32;
    fn SDL_GameControllerOpen(joystick_index: i32) -> *mut core::ffi::c_void;
    fn SDL_GameControllerClose(gc: *mut core::ffi::c_void);
    fn SDL_GameControllerName(gc: *mut core::ffi::c_void) -> *const core::ffi::c_char;
    fn SDL_GameControllerGetBindForButton(gc: *mut core::ffi::c_void, button: i32) -> SdlGcBind;
    fn SDL_GameControllerGetBindForAxis(gc: *mut core::ffi::c_void, axis: i32) -> SdlGcBind;
    fn SDL_GameControllerGetJoystick(gc: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn SDL_JoystickName(joy: *mut core::ffi::c_void) -> *const core::ffi::c_char;
    fn SDL_JoystickGetGUID(joy: *mut core::ffi::c_void) -> SdlJoystickGuid;
    fn SDL_JoystickGetGUIDString(guid: SdlJoystickGuid, psz: *mut core::ffi::c_char, len: i32);
    fn SDL_SetHintWithPriority(name: *const core::ffi::c_char, value: *const core::ffi::c_char, priority: i32) -> i32;
}

/// The pad's raw SDL *joystick* GUID as a 32-char lowercase hex string — the
/// identity Mednafen embeds in each binding (`joystick 0x<guid> ...`). Read from
/// the GameController's underlying joystick so it matches the same-backend device.
unsafe fn joystick_guid_string(gc: *mut core::ffi::c_void) -> Option<String> {
    let joy = SDL_GameControllerGetJoystick(gc);
    if joy.is_null() {
        return None;
    }
    let guid = SDL_JoystickGetGUID(joy);
    // SDL writes 32 hex chars + NUL; give it a 33-byte buffer.
    let mut buf = [0i8; 33];
    SDL_JoystickGetGUIDString(guid, buf.as_mut_ptr(), buf.len() as i32);
    let s = core::ffi::CStr::from_ptr(buf.as_ptr())
        .to_string_lossy()
        .into_owned();
    if s.is_empty() || s.chars().all(|c| c == '0') {
        None
    } else {
        Some(s)
    }
}

/// The pad's raw SDL *joystick* name (e.g. "Xbox Wireless Controller") — the
/// identity mGBA's **Qt** frontend keys its per-device `input-profile` section by
/// (`profileForType` → `SDL_JoystickName`). This is deliberately the *joystick*
/// name, not `SDL_GameControllerName`: the latter comes from the GameController
/// mapping DB and can read "Xbox 360 Controller" even when the joystick reports
/// "Xbox Wireless Controller", and mgba-qt looks up the joystick name. Read from
/// the GameController's underlying joystick so it matches the same-backend device.
unsafe fn joystick_name_string(gc: *mut core::ffi::c_void) -> Option<String> {
    let joy = SDL_GameControllerGetJoystick(gc);
    if joy.is_null() {
        return None;
    }
    let p = SDL_JoystickName(joy);
    if p.is_null() {
        return None;
    }
    let s = core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn hat_dir(mask: i32) -> Option<HatDir> {
    match mask {
        SDL_HAT_UP => Some(HatDir::Up),
        SDL_HAT_DOWN => Some(HatDir::Down),
        SDL_HAT_LEFT => Some(HatDir::Left),
        SDL_HAT_RIGHT => Some(HatDir::Right),
        _ => None,
    }
}

fn raw_from_bind(bind: SdlGcBind) -> Option<RawElement> {
    match bind.bind_type {
        BINDTYPE_BUTTON => Some(RawElement::Button(bind.a as u32)),
        BINDTYPE_AXIS => Some(RawElement::Axis(bind.a as u32)),
        BINDTYPE_HAT => hat_dir(bind.b).map(|d| RawElement::Hat(bind.a as u32, d)),
        _ => None, // BINDTYPE_NONE — element unbound on this device
    }
}

/// Match the SDL backend Arcadia uses to *launch* emulators (HIDAPI off, classic
/// joystick on for the xpadneo Bluetooth-Xbox case). The device GUID — and thus
/// the resolved mapping — can differ between backends, so the probe must enumerate
/// the pad the same way the emulator will, or the raw indices won't agree. Set via
/// SDL hints (SDL-internal, never leaking into child process env).
fn align_sdl_backend() {
    unsafe {
        // SDL_HINT_OVERRIDE = 2.
        let hidapi = b"SDL_JOYSTICK_HIDAPI\0";
        let classic = b"SDL_JOYSTICK_LINUX_CLASSIC\0";
        let zero = b"0\0";
        let one = b"1\0";
        SDL_SetHintWithPriority(hidapi.as_ptr().cast(), zero.as_ptr().cast(), 2);
        SDL_SetHintWithPriority(classic.as_ptr().cast(), one.as_ptr().cast(), 2);
    }
}

/// A probed pad: its normalized→raw element mapping plus the raw joystick GUID
/// (Mednafen's device identity), when available.
pub struct ProbedPad {
    pub mapping: PadMapping,
    pub guid: Option<String>,
    /// Raw SDL joystick name — mGBA's Qt frontend keys its per-device
    /// `input-profile` section by this (see [`joystick_name_string`]).
    pub name: Option<String>,
}

/// Probe the first connected SDL game controller and build its normalized→raw
/// element mapping plus joystick GUID. Returns `None` when SDL can't init or no
/// game controller is present (the caller then leaves Tier-B inputs unencoded
/// rather than guessing).
pub fn probe_first_controller() -> Option<ProbedPad> {
    align_sdl_backend();
    // SAFETY: each call below is a documented SDL2 C entry point; we own the
    // GameController handle for the duration and close it before quitting.
    unsafe {
        if SDL_InitSubSystem(SDL_INIT_GAMECONTROLLER) != 0 {
            tracing::warn!("SDL gamecontroller init failed; skipping controller probe");
            return None;
        }
        let result = (|| {
            let count = SDL_NumJoysticks();
            let index = (0..count).find(|&i| SDL_IsGameController(i) != 0)?;
            let gc = SDL_GameControllerOpen(index);
            if gc.is_null() {
                return None;
            }
            let name = {
                let p = SDL_GameControllerName(gc);
                if p.is_null() {
                    String::from("<unknown>")
                } else {
                    core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            };

            let mut mapping = PadMapping::new();
            // SDL_GameControllerButton 0..=14 (A..DPAD_RIGHT) — the standard set.
            for button in 0..=14i32 {
                if let Some(raw) = raw_from_bind(SDL_GameControllerGetBindForButton(gc, button)) {
                    mapping.set_button(button as u32, raw);
                }
            }
            // SDL_GameControllerAxis 0..=5 (LEFTX..TRIGGERRIGHT).
            for axis in 0..=5i32 {
                if let Some(raw) = raw_from_bind(SDL_GameControllerGetBindForAxis(gc, axis)) {
                    mapping.set_axis(axis as u32, raw);
                }
            }

            let guid = joystick_guid_string(gc);
            let joy_name = joystick_name_string(gc);
            SDL_GameControllerClose(gc);
            tracing::info!(controller = %name, joystick = ?joy_name, guid = ?guid, "probed SDL controller mapping for Tier-B write");
            if mapping.is_empty() {
                None
            } else {
                Some(ProbedPad { mapping, guid, name: joy_name })
            }
        })();
        SDL_QuitSubSystem(SDL_INIT_GAMECONTROLLER);
        result
    }
}
