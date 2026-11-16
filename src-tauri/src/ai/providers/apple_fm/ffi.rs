//! Swift 导出的 C 函数（`swift/Sources/AppleFm/AppleFm.swift`）和它们的安全包装。

use std::ffi::{c_char, c_void, CStr, CString};

use serde::Deserialize;
use tokio::sync::oneshot;

type Callback = unsafe extern "C" fn(context: *mut c_void, json: *const c_char);

extern "C" {
    fn hs_afm_status(language: *const c_char) -> *mut c_char;
    fn hs_afm_free(string: *mut c_char);
    fn hs_afm_count_tokens(
        instructions: *const c_char,
        prompt: *const c_char,
        context: *mut c_void,
        callback: Callback,
    ) -> u64;
    fn hs_afm_generate(
        instructions: *const c_char,
        prompt: *const c_char,
        max_tokens: u32,
        temperature: f64,
        context: *mut c_void,
        callback: Callback,
    ) -> u64;
    fn hs_afm_cancel(task: u64);
}

/// 问系统这台 Mac 能不能用模型，返回 Swift 交来的 JSON。
///
/// `language` 是 BCP 47 语言标签，比如 `zh-Hans`。
pub(super) fn status(language: &str) -> String {
    let language = CString::new(language).unwrap_or_default();
    // SAFETY: `language` 在调用期间有效。返回值是 Swift 用 `strdup` 分配的字符串，或者空指针。
    unsafe { take_string(hs_afm_status(language.as_ptr())) }
}

/// 让系统数 `instructions` 和 `prompt` 一共多少 token。
pub(super) fn count_tokens(instructions: &str, prompt: &str) -> Pending {
    let instructions = c_string(instructions);
    let prompt = c_string(prompt);
    // SAFETY: 两个字符串在调用期间有效，Swift 在返回前拷走。`context` 和 `on_done` 见 `start`。
    start(|context| unsafe {
        hs_afm_count_tokens(instructions.as_ptr(), prompt.as_ptr(), context, on_done)
    })
}

/// 让系统用 `instructions` 当系统提示词，回答 `prompt`。
///
/// `temperature` 是 `None` 时用系统默认。
pub(super) fn generate(
    instructions: &str,
    prompt: &str,
    max_tokens: u32,
    temperature: Option<f64>,
) -> Pending {
    let instructions = c_string(instructions);
    let prompt = c_string(prompt);
    let temperature = temperature.unwrap_or(f64::NAN);
    // SAFETY: 同 `count_tokens`。
    start(|context| unsafe {
        hs_afm_generate(
            instructions.as_ptr(),
            prompt.as_ptr(),
            max_tokens,
            temperature,
            context,
            on_done,
        )
    })
}

/// Swift 交来的结果，见 Swift 的 `Outcome`。
#[derive(Debug, Default, Deserialize)]
pub(super) struct Outcome {
    pub text: Option<String>,
    pub tokens: Option<u32>,
    pub error: Option<SystemError>,
}

/// 系统报的错误，见 Swift 的 `SystemError`。
#[derive(Debug, Deserialize)]
pub(super) struct SystemError {
    pub domain: String,
    pub code: i64,
    pub message: String,
}

/// 一个在 Swift 里跑的任务。还没等到结果就被丢掉时，让 Swift 取消它。
pub(super) struct Pending {
    task: u64,
    rx: oneshot::Receiver<String>,
    done: bool,
}

impl Pending {
    /// 等任务做完，返回 Swift 交来的结果。
    pub(super) async fn wait(mut self) -> Outcome {
        let json = (&mut self.rx).await.unwrap_or_default();
        self.done = true;
        serde_json::from_str(&json).unwrap_or_else(|e| Outcome {
            error: Some(SystemError {
                domain: "hindsight.apple_fm".to_string(),
                code: 0,
                message: format!("cannot read the reply from Swift: {e}"),
            }),
            ..Outcome::default()
        })
    }

    #[cfg(test)]
    pub(super) fn cancel(&self) {
        // SAFETY: 任务号是 Swift 给的。任务已经做完时 Swift 什么也不做。
        unsafe { hs_afm_cancel(self.task) }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.done {
            // SAFETY: 同 `cancel`。
            unsafe { hs_afm_cancel(self.task) }
        }
    }
}

/// 起一个 Swift 任务。`call` 把 `context` 和 [`on_done`] 交给 Swift，返回任务号。
fn start(call: impl FnOnce(*mut c_void) -> u64) -> Pending {
    let (tx, rx) = oneshot::channel::<String>();
    let context = Box::into_raw(Box::new(tx)).cast::<c_void>();
    let task = call(context);
    Pending {
        task,
        rx,
        done: false,
    }
}

/// Swift 做完任务时调用，每个任务一次。把结果交给等着的 [`Pending`]。
///
/// # Safety
///
/// `context` 是 [`start`] 交给 Swift 的指针，只能用一次。`json` 在调用期间有效。
unsafe extern "C" fn on_done(context: *mut c_void, json: *const c_char) {
    let tx = Box::from_raw(context.cast::<oneshot::Sender<String>>());
    let json = CStr::from_ptr(json).to_string_lossy().into_owned();
    // 等的一方已经被丢掉时发不出去，这时结果没人要。
    let _ = tx.send(json);
}

/// C 字符串不能含 NUL，有的话去掉。
fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
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
