//! IOKit power assertions.

use core_foundation::base::{CFRelease, TCFType};
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::{CFString, CFStringRef};

type IOPMAssertionID = u32;
type IOReturn = i32;

const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
const K_IORETURN_SUCCESS: IOReturn = 0;
/// `kIOPMAssertPreventUserIdleSystemSleep`: the system doesn't idle-sleep; the display still
/// may. We never hold a display assertion.
const PREVENT_USER_IDLE_SYSTEM_SLEEP: &str = "PreventUserIdleSystemSleep";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        level: u32,
        name: CFStringRef,
        id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(id: IOPMAssertionID) -> IOReturn;
    fn IOPMAssertionCopyProperties(id: IOPMAssertionID) -> CFDictionaryRef;
}

/// Creates a `PreventUserIdleSystemSleep` assertion owned by this process. The OS releases it
/// when the process exits, including on a crash.
pub fn prevent_idle_sleep(name: &str) -> Result<u32, IOReturn> {
    let kind = CFString::from_static_string(PREVENT_USER_IDLE_SYSTEM_SLEEP);
    let name = CFString::new(name);
    let mut id: IOPMAssertionID = 0;
    // SAFETY: both strings are valid CFStrings for the duration of the call, and `id` is a
    // valid out pointer.
    let ret = unsafe {
        IOPMAssertionCreateWithName(
            kind.as_concrete_TypeRef(),
            K_IOPM_ASSERTION_LEVEL_ON,
            name.as_concrete_TypeRef(),
            &mut id,
        )
    };
    if ret == K_IORETURN_SUCCESS {
        Ok(id)
    } else {
        Err(ret)
    }
}

/// Releases an assertion this process created.
pub fn release(id: u32) -> Result<(), IOReturn> {
    // SAFETY: plain value call; an unknown id yields an error code, not UB.
    let ret = unsafe { IOPMAssertionRelease(id) };
    if ret == K_IORETURN_SUCCESS {
        Ok(())
    } else {
        Err(ret)
    }
}

/// Whether the assertion is still held.
pub fn is_held(id: u32) -> bool {
    // SAFETY: returns a +1 dictionary or null; we release it right away.
    let props = unsafe { IOPMAssertionCopyProperties(id) };
    if props.is_null() {
        return false;
    }
    // SAFETY: `props` is a non-null CF object we own.
    unsafe { CFRelease(props.cast()) };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_release() {
        let id = prevent_idle_sleep("Vigilia test assertion").unwrap();
        assert!(is_held(id));
        release(id).unwrap();
        assert!(!is_held(id));
        assert!(release(id).is_err());
    }
}
