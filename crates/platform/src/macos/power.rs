//! Reading and changing macOS power state: `pmset disablesleep` (through the sudoers rule
//! installed by `vigilia install`) and battery readings from IOPS.

use std::process::Command;

use vigilia_protocol::{PowerSnapshot, ThermalLevel};

use crate::{PlatformError, Result};

const PMSET: &str = "/usr/bin/pmset";
const SUDO: &str = "/usr/bin/sudo";

/// Whether sleep is currently disabled, parsed from `pmset -g` output. The key is absent
/// (meaning `0`) until `disablesleep` has been set.
pub fn parse_sleep_disabled(pmset_g: &str) -> bool {
    pmset_g
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some("SleepDisabled"), value) => value,
                _ => None,
            }
        })
        .any(|value| value == "1")
}

pub fn sleep_disabled() -> Result<bool> {
    let output = Command::new(PMSET)
        .arg("-g")
        .output()
        .map_err(|e| os("pmset -g", e))?;
    if !output.status.success() {
        return Err(PlatformError::Os {
            op: "pmset -g",
            detail: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(parse_sleep_disabled(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Sets `disablesleep` and returns the value it had before. Goes through `sudo -n`, so it
/// only works with the sudoers rule from `vigilia install`; without it, sudo fails at once
/// instead of prompting.
pub fn set_sleep_disabled(disabled: bool) -> Result<bool> {
    let before = sleep_disabled()?;
    if before == disabled {
        return Ok(before);
    }
    let output = Command::new(SUDO)
        .args([
            "-n",
            PMSET,
            "-a",
            "disablesleep",
            if disabled { "1" } else { "0" },
        ])
        .output()
        .map_err(|e| os("sudo pmset", e))?;
    if !output.status.success() {
        return Err(PlatformError::Os {
            op: "pmset disablesleep",
            detail: format!(
                "sudo refused (run `vigilia install` first): {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(before)
}

/// Battery percent from current/max capacity, or `None` without a battery.
pub fn battery_percent(current: Option<i64>, max: Option<i64>) -> Option<u8> {
    match (current, max) {
        (Some(current), Some(max)) if max > 0 && current >= 0 => {
            u8::try_from((current.saturating_mul(100) / max).clamp(0, 100)).ok()
        }
        _ => None,
    }
}

/// Maps `ProcessInfo.thermalState` values to [`ThermalLevel`].
pub fn thermal_from_ns_value(value: isize) -> ThermalLevel {
    match value {
        1 => ThermalLevel::Fair,
        2 => ThermalLevel::Serious,
        3 => ThermalLevel::Critical,
        _ => ThermalLevel::Nominal,
    }
}

/// The current battery/heat reading: IOPS for the battery, `ProcessInfo` for heat.
pub fn power_snapshot() -> PowerSnapshot {
    let (battery_percent, on_ac) = battery_reading().unwrap_or((None, true));
    PowerSnapshot {
        battery_percent,
        on_ac,
        thermal: thermal_level(),
    }
}

fn os(op: &'static str, e: std::io::Error) -> PlatformError {
    PlatformError::Os {
        op,
        detail: e.to_string(),
    }
}

// --- IOPS battery reading -------------------------------------------------

use core_foundation::array::CFArrayRef;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::{CFString, CFStringRef};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, source: CFTypeRef) -> CFDictionaryRef;
    fn IOPSNotificationCreateRunLoopSource(
        callback: unsafe extern "C" fn(*mut std::ffi::c_void),
        context: *mut std::ffi::c_void,
    ) -> core_foundation::runloop::CFRunLoopSourceRef;
}

pub(super) fn iops_notification_source(
    callback: unsafe extern "C" fn(*mut std::ffi::c_void),
    context: *mut std::ffi::c_void,
) -> core_foundation::runloop::CFRunLoopSourceRef {
    // SAFETY: the caller keeps `context` alive for the source's lifetime.
    unsafe { IOPSNotificationCreateRunLoopSource(callback, context) }
}

fn dict_value(
    dict: &core_foundation::dictionary::CFDictionary,
    key: &str,
) -> Option<*const std::ffi::c_void> {
    let (keys, values) = dict.get_keys_and_values();
    for (k, v) in keys.into_iter().zip(values) {
        // SAFETY: keys of an IOPS description are CFStrings owned by the dictionary.
        let k = unsafe { CFString::wrap_under_get_rule(k as CFStringRef) };
        if k.to_string().eq(key) {
            return Some(v);
        }
    }
    None
}

fn dict_i64(dict: &core_foundation::dictionary::CFDictionary, key: &str) -> Option<i64> {
    let value = dict_value(dict, key)?;
    // SAFETY: IOPS capacity values are CFNumbers owned by the dictionary.
    let number = unsafe { CFNumber::wrap_under_get_rule(value as CFNumberRef) };
    number.to_i64()
}

fn dict_string(dict: &core_foundation::dictionary::CFDictionary, key: &str) -> Option<String> {
    let value = dict_value(dict, key)?;
    // SAFETY: IOPS state values are CFStrings owned by the dictionary.
    let string = unsafe { CFString::wrap_under_get_rule(value as CFStringRef) };
    Some(string.to_string())
}

fn battery_reading() -> Option<(Option<u8>, bool)> {
    // SAFETY: all objects returned by IOPS are +1 and released via their CF wrappers.
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return None;
        }
        let blob_ref = core_foundation::base::CFType::wrap_under_create_rule(blob);
        let list = IOPSCopyPowerSourcesList(blob_ref.as_CFTypeRef());
        if list.is_null() {
            return None;
        }
        let list = core_foundation::array::CFArray::wrap_under_create_rule(list);
        let mut result = None;
        for source in list.iter() {
            let desc = IOPSGetPowerSourceDescription(blob_ref.as_CFTypeRef(), *source);
            if desc.is_null() {
                continue;
            }
            // IOPSGetPowerSourceDescription is a "get" rule: borrowed, not owned.
            let desc = core_foundation::dictionary::CFDictionary::wrap_under_get_rule(desc);
            let state = dict_string(&desc, "Power Source State");
            // The internal battery is the source that can run on battery power.
            if dict_i64(&desc, "Max Capacity").is_some() {
                result = Some((
                    battery_percent(
                        dict_i64(&desc, "Current Capacity"),
                        dict_i64(&desc, "Max Capacity"),
                    ),
                    state.as_deref() == Some("AC Power"),
                ));
                break;
            }
            result.get_or_insert((None, state.as_deref() == Some("AC Power")));
        }
        result
    }
}

// --- ProcessInfo thermal state, through the Objective-C runtime -----------

use std::ffi::{c_char, c_void};

type ObjCId = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> ObjCId;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
}

fn thermal_level() -> ThermalLevel {
    // SAFETY: NSProcessInfo +processInfo is a singleton; thermalState is a plain NSInteger
    // getter. objc_msgSend is cast to the precise signature for each call.
    unsafe {
        let class = objc_getClass(c"NSProcessInfo".as_ptr());
        if class.is_null() {
            return ThermalLevel::Nominal;
        }
        let process_info: unsafe extern "C" fn(ObjCId, *mut c_void) -> ObjCId =
            std::mem::transmute(objc_msg_send as *const c_void);
        let instance = process_info(class, sel_registerName(c"processInfo".as_ptr()));
        if instance.is_null() {
            return ThermalLevel::Nominal;
        }
        let thermal_state: unsafe extern "C" fn(ObjCId, *mut c_void) -> isize =
            std::mem::transmute(objc_msg_send as *const c_void);
        thermal_from_ns_value(thermal_state(
            instance,
            sel_registerName(c"thermalState".as_ptr()),
        ))
    }
}

unsafe extern "C" {
    #[link_name = "objc_msgSend"]
    fn objc_msg_send();
}

pub(super) fn objc_class(name: &std::ffi::CStr) -> ObjCId {
    // SAFETY: plain runtime lookup.
    unsafe { objc_getClass(name.as_ptr()) }
}

pub(super) fn objc_sel(name: &std::ffi::CStr) -> *mut c_void {
    // SAFETY: plain runtime lookup.
    unsafe { sel_registerName(name.as_ptr()) }
}

pub(super) fn msg_send_id(receiver: ObjCId, sel: *mut c_void) -> ObjCId {
    // SAFETY: caller guarantees the selector takes no arguments and returns an object.
    unsafe {
        let f: unsafe extern "C" fn(ObjCId, *mut c_void) -> ObjCId =
            std::mem::transmute(objc_msg_send as *const c_void);
        f(receiver, sel)
    }
}

pub(super) fn msg_send_id_id(
    receiver: ObjCId,
    sel: *mut c_void,
    first: ObjCId,
    second: ObjCId,
    third: ObjCId,
    fourth: ObjCId,
) -> ObjCId {
    // SAFETY: caller guarantees the selector takes four object arguments.
    unsafe {
        let f: unsafe extern "C" fn(ObjCId, *mut c_void, ObjCId, ObjCId, ObjCId, ObjCId) -> ObjCId =
            std::mem::transmute(objc_msg_send as *const c_void);
        f(receiver, sel, first, second, third, fourth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sleep_disabled() {
        assert!(!parse_sleep_disabled(" sleep  1\n"));
        assert!(parse_sleep_disabled(" SleepDisabled 1\n sleep 1\n"));
        assert!(!parse_sleep_disabled(" SleepDisabled 0\n"));
    }

    #[test]
    fn battery_percent_math() {
        assert_eq!(battery_percent(Some(42), Some(100)), Some(42));
        assert_eq!(battery_percent(Some(1), Some(3)), Some(33));
        assert_eq!(battery_percent(None, Some(100)), None);
        assert_eq!(battery_percent(Some(5), Some(0)), None);
        assert_eq!(battery_percent(Some(150), Some(100)), Some(100));
    }

    #[test]
    fn thermal_mapping() {
        assert_eq!(thermal_from_ns_value(0), ThermalLevel::Nominal);
        assert_eq!(thermal_from_ns_value(3), ThermalLevel::Critical);
        assert_eq!(thermal_from_ns_value(99), ThermalLevel::Nominal);
    }

    #[test]
    fn reads_a_power_snapshot() {
        let snap = power_snapshot();
        // On any Mac this is well-formed; a desktop reports no battery.
        assert!(snap.battery_percent.is_none_or(|p| p <= 100));
    }
}
