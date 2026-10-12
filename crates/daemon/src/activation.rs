//! launchd socket activation. With the `LaunchAgent` installed, launchd owns the socket and starts
//! the service on the first connection; the service picks the socket up here.

use std::os::unix::net::UnixListener;

/// The `Sockets` key name in the `LaunchAgent` plist.
#[cfg(target_os = "macos")]
const SOCKET_NAME: &std::ffi::CStr = c"Listeners";

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn launch_activate_socket(
        name: *const libc::c_char,
        fds: *mut *mut libc::c_int,
        cnt: *mut libc::size_t,
    ) -> libc::c_int;
}

/// The listener launchd passed us, or `None` when we weren't started by launchd.
#[cfg(target_os = "macos")]
pub fn inherited_listener() -> Option<UnixListener> {
    use std::os::fd::{FromRawFd, RawFd};

    let mut fds: *mut libc::c_int = std::ptr::null_mut();
    let mut count: libc::size_t = 0;
    // SAFETY: valid out pointers; on success launchd hands us a malloc'd array we free below.
    let ret = unsafe { launch_activate_socket(SOCKET_NAME.as_ptr(), &raw mut fds, &raw mut count) };
    if ret != 0 || fds.is_null() {
        return None;
    }
    // SAFETY: launchd returned `count` valid fds in `fds`.
    let all: Vec<RawFd> = unsafe { std::slice::from_raw_parts(fds, count) }.to_vec();
    // SAFETY: the array was allocated with malloc by launchd.
    unsafe { libc::free(fds.cast()) };
    let mut all = all.into_iter();
    let first = all.next()?;
    for extra in all {
        // SAFETY: we own these fds and don't use them.
        unsafe { libc::close(extra) };
    }
    // SAFETY: `first` is a listening socket fd that we now own.
    Some(unsafe { UnixListener::from_raw_fd(first) })
}

/// systemd socket activation arrives with the Linux backend (step 7).
#[cfg(not(target_os = "macos"))]
pub fn inherited_listener() -> Option<UnixListener> {
    None
}
