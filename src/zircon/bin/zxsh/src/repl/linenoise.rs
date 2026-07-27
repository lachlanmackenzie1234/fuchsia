// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use bstr::{BStr, BString, ByteSlice};
use fuchsia_sync::Mutex;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

#[repr(C)]
struct linenoiseCompletions {
    len: usize,
    cvec: *mut *mut c_char,
}

#[cfg(test)]
impl linenoiseCompletions {
    fn new() -> Self {
        Self { len: 0, cvec: std::ptr::null_mut() }
    }
}

unsafe extern "C" {
    fn linenoise(prompt: *const c_char) -> *mut c_char;
    fn linenoiseFree(ptr: *mut c_void);
    fn linenoiseHistoryAdd(line: *const c_char) -> c_int;
    fn linenoiseHistorySetMaxLen(len: c_int) -> c_int;
    fn linenoiseAddCompletion(lc: *mut linenoiseCompletions, str_: *const c_char);
    fn linenoiseSetCompletionCallback(
        cb: Option<unsafe extern "C" fn(*const c_char, *mut linenoiseCompletions)>,
    );
}

/// A list of completion candidates provided to linenoise.
pub struct Completions {
    raw: *mut linenoiseCompletions,
}

impl Completions {
    /// Adds a completion candidate to the list.
    ///
    /// # Arguments
    ///
    /// * `completion` - The completion candidate as a byte slice. It must not contain null bytes.
    pub fn add(&mut self, completion: &[u8]) {
        if let Ok(c_str) = CString::new(completion) {
            unsafe {
                linenoiseAddCompletion(self.raw, c_str.as_ptr());
            }
        }
    }
}

static USER_CALLBACK: Mutex<Option<fn(&BStr, &mut Completions)>> = Mutex::new(None);

unsafe extern "C" fn raw_callback(line: *const c_char, lc: *mut linenoiseCompletions) {
    if line.is_null() {
        return;
    }
    let c_line = unsafe { CStr::from_ptr(line) };
    let line_bytes = c_line.to_bytes();
    let cb = USER_CALLBACK.lock().clone();
    if let Some(cb) = cb {
        let mut comps = Completions { raw: lc };
        let line_bstr = BStr::new(line_bytes);
        cb(line_bstr, &mut comps);
    }
}

/// Sets the callback function that linenoise will call to retrieve completions.
///
/// The callback receives the current input line as a `BStr` and a mutable reference
/// to a `Completions` object where it can add completion candidates.
pub fn set_completion_callback(cb: fn(&BStr, &mut Completions)) {
    *USER_CALLBACK.lock() = Some(cb);
    unsafe {
        linenoiseSetCompletionCallback(Some(raw_callback));
    }
}

/// Prompts the user for input and reads a line.
///
/// # Arguments
///
/// * `prompt` - The prompt to display to the user.
///
/// # Returns
///
/// * `Some(BString)` - The line entered by the user (excluding the newline character).
/// * `None` - If the user canceled the input (e.g., Ctrl-C/D) or an error occurred.
pub fn readline(prompt: &BStr) -> Option<BString> {
    let c_prompt = CString::new(prompt.as_bytes().to_vec()).ok()?;
    let c_line = unsafe { linenoise(c_prompt.as_ptr()) };
    if c_line.is_null() {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(c_line) }.to_bytes().to_vec();
    unsafe { linenoiseFree(c_line as *mut c_void) };
    Some(BString::from(bytes))
}

/// Adds a line to the command history.
///
/// # Arguments
///
/// * `line` - The line to add to the history.
///
/// # Returns
///
/// * `true` - If the line was successfully added.
/// * `false` - If adding failed (e.g., if the line contained null bytes).
pub fn history_add(line: &BStr) -> bool {
    if let Ok(c_line) = CString::new(line.as_bytes().to_vec()) {
        unsafe { linenoiseHistoryAdd(c_line.as_ptr()) != 0 }
    } else {
        false
    }
}

/// Sets the maximum number of lines to keep in the command history.
///
/// # Arguments
///
/// * `len` - The maximum history length.
///
/// # Returns
///
/// * `true` - If the history limit was successfully updated.
/// * `false` - If updating failed.
pub fn history_set_max_len(len: usize) -> bool {
    unsafe { linenoiseHistorySetMaxLen(len as c_int) != 0 }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(crate) struct TestCompletionsGuard {
        lc: linenoiseCompletions,
    }

    impl TestCompletionsGuard {
        pub(crate) fn new() -> Self {
            Self { lc: linenoiseCompletions::new() }
        }

        pub(crate) fn completions(&mut self) -> Completions {
            Completions { raw: &mut self.lc }
        }

        pub(crate) fn items(&self) -> Vec<BString> {
            let mut res = Vec::new();
            if !self.lc.cvec.is_null() {
                for i in 0..self.lc.len {
                    unsafe {
                        let ptr = *self.lc.cvec.add(i);
                        if !ptr.is_null() {
                            let cstr = CStr::from_ptr(ptr);
                            res.push(BString::from(cstr.to_bytes()));
                        }
                    }
                }
            }
            res
        }
    }

    impl Drop for TestCompletionsGuard {
        fn drop(&mut self) {
            if !self.lc.cvec.is_null() {
                for i in 0..self.lc.len {
                    unsafe {
                        let ptr = *self.lc.cvec.add(i);
                        if !ptr.is_null() {
                            libc::free(ptr as *mut libc::c_void);
                        }
                    }
                }
                unsafe {
                    libc::free(self.lc.cvec as *mut libc::c_void);
                }
            }
        }
    }

    #[test]
    fn test_history_operations() {
        assert!(history_set_max_len(10));
        assert!(history_add(BStr::new("echo hello")));
        assert!(!history_add(BStr::new(b"invalid\0line")));
    }

    #[test]
    fn test_completions_add() {
        let mut guard = TestCompletionsGuard::new();
        let mut comps = guard.completions();

        comps.add(b"first_completion");
        comps.add(b"invalid\0completion");
        comps.add(b"second_completion");

        let items = guard.items();
        assert_eq!(
            items,
            vec![BString::from("first_completion"), BString::from("second_completion")]
        );
    }

    static CALLBACK_CALLED: AtomicBool = AtomicBool::new(false);

    fn dummy_callback(line: &BStr, comps: &mut Completions) {
        CALLBACK_CALLED.store(true, Ordering::SeqCst);
        comps.add(line.as_bytes());
    }

    #[test]
    fn test_completion_callback_and_raw_callback() {
        set_completion_callback(dummy_callback);

        // Test null line in raw_callback (returns early without calling callback)
        unsafe {
            raw_callback(std::ptr::null(), std::ptr::null_mut());
        }
        assert!(!CALLBACK_CALLED.load(Ordering::SeqCst));

        // Test valid line in raw_callback
        let mut guard = TestCompletionsGuard::new();
        let line_c = CString::new("test_cmd").unwrap();
        unsafe {
            raw_callback(line_c.as_ptr(), &mut guard.lc);
        }
        assert!(CALLBACK_CALLED.load(Ordering::SeqCst));
        assert_eq!(guard.items(), vec![BString::from("test_cmd")]);
    }

    #[test]
    fn test_readline_null_prompt() {
        assert_eq!(readline(BStr::new(b"prompt\0invalid")), None);
    }
}
