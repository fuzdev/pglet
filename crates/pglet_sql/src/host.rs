//! Calls out to the JS host from inside a statement, on wasm.
//!
//! Every such call runs user-reachable JS — `Date.now` and `Math.random` are globals a
//! page can replace, and a bound parameter's getters and `toJSON` are the caller's own
//! code — and a JS exception that crosses a wasm frame unwinds it with no destructor run
//! and the shadow-stack pointer left where the frame had it, which is why the module
//! refuses every call after one (`pglet_wasm`'s `call` module). So a call out is made
//! through an import that **catches** (`Reflect.get` / `Reflect.apply`), and returns the
//! exception as an error rather than unwinding.
//!
//! It is also marked while it runs ([`call`]): JS the module called can call back into
//! the module, and a call that finds another still running is told apart from one that
//! finds a call that never returned by whether that other call is out in the host now.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set while a statement is out in the host ([`call`]).
static IN_HOST_CALL: AtomicBool = AtomicBool::new(false);

/// Run `f` — one call out to the JS host — marked as out in the host while it runs.
///
/// `f` must not unwind: it makes its JS calls through imports that catch.
pub fn call<T>(f: impl FnOnce() -> T) -> T {
    let outer = IN_HOST_CALL.swap(true, Ordering::Relaxed);
    let out = f();
    IN_HOST_CALL.store(outer, Ordering::Relaxed);
    out
}

/// Whether a statement is out in the host now — so a call into the module arriving now
/// was made from inside it.
#[must_use]
pub fn in_call() -> bool {
    IN_HOST_CALL.load(Ordering::Relaxed)
}

/// Call the global function `object.method()` through imports that catch, returning
/// what it answered as a number, `NaN` refused.
#[cfg(target_arch = "wasm32")]
fn global_number(object: &str, method: &str) -> crate::Result<f64> {
    use js_sys::{Array, Function, JsString, Reflect};
    let what = || format!("{object}.{method}()");
    let threw = |_| crate::Error::Execution(format!("{} threw", what()));
    call(|| {
        let target =
            Reflect::get(&js_sys::global(), JsString::from(object).as_ref()).map_err(threw)?;
        let function = Reflect::get(&target, JsString::from(method).as_ref()).map_err(threw)?;
        if !function.is_function() {
            return Err(crate::Error::Execution(format!(
                "{} is not a function",
                what()
            )));
        }
        Reflect::apply(&Function::from(function), &target, &Array::new())
            .map_err(threw)?
            .as_f64()
            // `NaN` is a JS number and no answer: cast, it would read as zero.
            .filter(|answer| !answer.is_nan())
            .ok_or_else(|| crate::Error::Execution(format!("{} did not answer a number", what())))
    })
}

/// `Date.now()`, caught.
///
/// # Errors
///
/// `Error::Execution` when it throws, or answers `NaN` or a value that is not a number.
#[cfg(target_arch = "wasm32")]
pub fn date_now() -> crate::Result<f64> {
    global_number("Date", "now")
}

/// `Math.random()`, caught.
///
/// # Errors
///
/// `Error::Execution` when it throws, or answers `NaN` or a value that is not a number.
#[cfg(target_arch = "wasm32")]
pub fn math_random() -> crate::Result<f64> {
    global_number("Math", "random")
}
