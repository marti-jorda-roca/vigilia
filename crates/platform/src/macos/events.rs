//! Lid, battery and heat events from OS notifications, on one `CFRunLoop` thread. Nothing here
//! polls: `IOKit` interest notifications cover the lid, `IOPSNotificationCreateRunLoopSource`
//! covers power sources, and an `NSProcessInfoThermalStateDidChangeNotification` observer
//! covers heat. Every callback re-reads the current state and forwards only real changes.

use std::ffi::{c_char, c_void};
use std::sync::Mutex;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBooleanRef;
use core_foundation::runloop::{CFRunLoop, CFRunLoopSourceRef, kCFRunLoopDefaultMode};
use core_foundation::string::CFString;
use tokio::sync::mpsc;
use vigilia_protocol::{LidState, PowerSnapshot};

use super::power;

type IoService = u32;
type IoNotification = u32;
type NotificationPort = *mut c_void;
type AllocatorRef = CFTypeRef;

const K_IO_MAIN_PORT_DEFAULT: u32 = 0;
const K_IO_GENERAL_INTEREST: &core::ffi::CStr = c"IOGeneralInterest";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFTypeRef;
    fn IOServiceGetMatchingService(main_port: u32, matching: CFTypeRef) -> IoService;
    fn IORegistryEntryCreateCFProperty(
        entry: IoService,
        key: core_foundation::string::CFStringRef,
        allocator: AllocatorRef,
        options: u32,
    ) -> CFTypeRef;
    fn IOObjectRelease(object: u32) -> i32;
    fn IONotificationPortCreate(main_port: u32) -> NotificationPort;
    fn IONotificationPortGetRunLoopSource(port: NotificationPort) -> CFRunLoopSourceRef;
    fn IOServiceAddInterestNotification(
        port: NotificationPort,
        service: IoService,
        interest_type: *const c_char,
        callback: unsafe extern "C" fn(*mut c_void, IoService, u32, *mut c_void),
        ref_con: *mut c_void,
        notification: *mut IoNotification,
    ) -> i32;
    fn CFBooleanGetValue(value: CFBooleanRef) -> bool;
}

/// Whether the lid is closed right now, from `AppleClamshellState` on `IOPMrootDomain`.
pub fn lid_closed() -> Option<bool> {
    // SAFETY: standard IOKit registry lookup; every object obtained is released.
    unsafe {
        let matching = IOServiceMatching(c"IOPMrootDomain".as_ptr());
        if matching.is_null() {
            return None;
        }
        let service = IOServiceGetMatchingService(K_IO_MAIN_PORT_DEFAULT, matching);
        if service == 0 {
            return None;
        }
        let key = CFString::from_static_string("AppleClamshellState");
        let value = IORegistryEntryCreateCFProperty(
            service,
            key.as_concrete_TypeRef(),
            std::ptr::null_mut(),
            0,
        );
        IOObjectRelease(service);
        if value.is_null() {
            return None;
        }
        let closed = CFBooleanGetValue(value.cast());
        CFRelease(value);
        Some(closed)
    }
}

pub fn lid_state() -> Option<LidState> {
    lid_closed().map(|closed| {
        if closed {
            LidState::Closed
        } else {
            LidState::Open
        }
    })
}

struct Watcher {
    lid_tx: mpsc::UnboundedSender<LidState>,
    power_tx: mpsc::UnboundedSender<PowerSnapshot>,
    last_lid: Mutex<Option<LidState>>,
    last_power: Mutex<Option<PowerSnapshot>>,
}

impl Watcher {
    fn publish_current(&self) {
        if let Some(state) = lid_state() {
            let mut last = self.last_lid.lock().unwrap();
            if *last != Some(state) {
                *last = Some(state);
                drop(last);
                let _ = self.lid_tx.send(state);
            }
        }
        let snap = power::power_snapshot();
        let mut last = self.last_power.lock().unwrap();
        if *last != Some(snap) {
            *last = Some(snap);
            drop(last);
            let _ = self.power_tx.send(snap);
        }
    }
}

unsafe extern "C" fn on_system_change(
    ref_con: *mut c_void,
    _service: IoService,
    _message_type: u32,
    _message_argument: *mut c_void,
) {
    // SAFETY: `ref_con` is the leaked `Watcher` created in `spawn`.
    let watcher = unsafe { &*(ref_con.cast::<Watcher>()) };
    watcher.publish_current();
}

unsafe extern "C" fn on_power_change(ref_con: *mut c_void) {
    // SAFETY: as in `on_system_change`.
    let watcher = unsafe { &*(ref_con.cast::<Watcher>()) };
    watcher.publish_current();
}

/// Starts the notification thread. The senders are dropped with the platform, which stops
/// the daemon from receiving further events (the thread itself lives for the process).
pub fn spawn(
    lid_tx: mpsc::UnboundedSender<LidState>,
    power_tx: mpsc::UnboundedSender<PowerSnapshot>,
) {
    let watcher: &'static Watcher = Box::leak(Box::new(Watcher {
        lid_tx,
        power_tx,
        last_lid: Mutex::new(None),
        last_power: Mutex::new(None),
    }));
    std::thread::Builder::new()
        .name("vigilia-events".into())
        .spawn(move || run(watcher))
        .ok();
}

fn run(watcher: &'static Watcher) {
    watcher.publish_current();
    register_thermal_observer(watcher);
    // SAFETY: sets up IOKit/IOPS runloop sources whose callbacks receive `watcher`, which is
    // leaked and therefore outlives the runloop. This thread then runs the runloop forever.
    unsafe {
        let runloop = CFRunLoop::get_current();
        let power_source = power::iops_notification_source(
            on_power_change,
            std::ptr::from_ref(watcher).cast_mut().cast::<c_void>(),
        );
        if !power_source.is_null() {
            runloop.add_source(
                &core_foundation::runloop::CFRunLoopSource::wrap_under_get_rule(power_source),
                kCFRunLoopDefaultMode,
            );
        }
        let port = IONotificationPortCreate(K_IO_MAIN_PORT_DEFAULT);
        if !port.is_null() {
            let matching = IOServiceMatching(c"IOPMrootDomain".as_ptr());
            let service = IOServiceGetMatchingService(K_IO_MAIN_PORT_DEFAULT, matching);
            if service != 0 {
                let mut notification: IoNotification = 0;
                let ret = IOServiceAddInterestNotification(
                    port,
                    service,
                    K_IO_GENERAL_INTEREST.as_ptr(),
                    on_system_change,
                    std::ptr::from_ref(watcher).cast_mut().cast::<c_void>(),
                    &raw mut notification,
                );
                if ret != 0 {
                    tracing::warn!(code = ret, "could not subscribe to lid notifications");
                }
                IOObjectRelease(service);
            }
            let source = IONotificationPortGetRunLoopSource(port);
            if !source.is_null() {
                runloop.add_source(
                    &core_foundation::runloop::CFRunLoopSource::wrap_under_get_rule(source),
                    kCFRunLoopDefaultMode,
                );
            }
        }
        CFRunLoop::run_current();
    }
}

// --- NSProcessInfo thermal notification, via a tiny dynamic ObjC class -----

type ObjCId = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_allocateClassPair(
        superclass: ObjCId,
        name: *const c_char,
        extra_bytes: usize,
    ) -> ObjCId;
    fn objc_registerClassPair(cls: ObjCId);
    fn class_addMethod(
        cls: ObjCId,
        sel: *mut c_void,
        imp: unsafe extern "C" fn(ObjCId, *mut c_void, ObjCId),
        types: *const c_char,
    ) -> bool;
}

static THERMAL_WATCHER: Mutex<Option<&'static Watcher>> = Mutex::new(None);

unsafe extern "C" fn on_thermal_changed(_self: ObjCId, _sel: *mut c_void, _note: ObjCId) {
    if let Some(watcher) = *THERMAL_WATCHER.lock().unwrap() {
        watcher.publish_current();
    }
}

fn register_thermal_observer(watcher: &'static Watcher) {
    *THERMAL_WATCHER.lock().unwrap() = Some(watcher);
    // SAFETY: creates a process-lifetime NSObject subclass with one method, then registers
    // an instance for the thermal-state notification. All objects are intentionally leaked.
    unsafe {
        let ns_object = power_objc_class(c"NSObject");
        if ns_object.is_null() {
            return;
        }
        let mut cls = power_objc_class(c"VigiliaThermalObserver");
        if cls.is_null() {
            cls = objc_allocateClassPair(ns_object, c"VigiliaThermalObserver".as_ptr(), 0);
            if cls.is_null() {
                return;
            }
            class_addMethod(
                cls,
                power::objc_sel(c"vigiliaThermalChanged:"),
                on_thermal_changed,
                c"v@:@".as_ptr(),
            );
            objc_registerClassPair(cls);
        }
        let observer = power::msg_send_id(
            power::msg_send_id(cls, power::objc_sel(c"alloc")),
            power::objc_sel(c"init"),
        );
        let center_class = power_objc_class(c"NSNotificationCenter");
        let center = power::msg_send_id(center_class, power::objc_sel(c"defaultCenter"));
        let string_class = power_objc_class(c"NSString");
        let alloc_str = power::msg_send_id(string_class, power::objc_sel(c"alloc"));
        let name = {
            let f: unsafe extern "C" fn(ObjCId, *mut c_void, *const c_char) -> ObjCId =
                std::mem::transmute(objc_msg_send_ptr());
            f(
                alloc_str,
                power::objc_sel(c"initWithUTF8String:"),
                c"NSProcessInfoThermalStateDidChangeNotification".as_ptr(),
            )
        };
        power::msg_send_id_id(
            center,
            power::objc_sel(c"addObserver:selector:name:object:"),
            observer,
            power::objc_sel(c"vigiliaThermalChanged:"),
            name,
            std::ptr::null_mut(),
        );
    }
}

fn power_objc_class(name: &core::ffi::CStr) -> ObjCId {
    power::objc_class(name)
}

fn objc_msg_send_ptr() -> *const c_void {
    // SAFETY: address of the objc_msgSend function; callers transmute to an exact signature.
    objc_msg_send_addr as *const c_void
}

unsafe extern "C" {
    #[link_name = "objc_msgSend"]
    fn objc_msg_send_addr();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lid_state_is_readable() {
        // A desktop Mac reports no clamshell; a laptop reports open or closed. Either is fine.
        let _ = lid_state();
    }
}
