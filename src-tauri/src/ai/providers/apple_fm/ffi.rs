//! Swift 导出的 C 函数（`swift/Sources/AppleFm/AppleFm.swift`）和它们的安全包装。

use std::ffi::{c_char, CStr, CString};

extern "C" {
    fn hs_afm_status(language: *const c_char) -> *mut c_char;
    fn hs_afm_free(string: *mut c_char);
}

/// 问系统这台 Mac 能不能用模型，返回 Swift 交来的 JSON。
///
/// `language` 是 BCP 47 语言标签，比如 `zh-Hans`。
pub(super) fn status(language: &str) -> String {
    let language = CString::new(language).unwrap_or_default();
    // SAFETY: `language` 在调用期间有效。返回值是 Swift 用 `strdup` 分配的字符串，或者空指针。
    unsafe { take_string(hs_afm_status(language.as_ptr())) }
}

/// 拷出 Swift 交来的字符串，再交回 Swift 释放。空指针返回空字符串。
///
/// # Safety
///
/// `string` 是空指针，或者是 Swift 用 `strdup` 分配、还没释放的字符串。
unsafe fn take_string(string: *mut c_char) -> String {
    if string.is_null() {
        return String::new();
    }
    let out = CStr::from_ptr(string).to_string_lossy().into_owned();
    hs_afm_free(string);
    out
}
