//! The session-change doorbell: a Darwin notification posted with `notify_post` (libSystem).

unsafe extern "C" {
    fn notify_post(name: *const std::ffi::c_char) -> u32;
}

/// Posts `com.vigilia.session-changed`. Best-effort: with no listener there is nothing to do.
pub fn post_session_changed() {
    // SAFETY: the name is a static NUL-terminated C string and `notify_post` only reads it.
    unsafe { notify_post(c"com.vigilia.session-changed".as_ptr()) };
}
