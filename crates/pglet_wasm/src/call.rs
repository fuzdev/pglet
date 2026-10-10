//! Whether a host exception has left this module in the middle of a call.
//!
//! A statement runs on two stacks here: the linear-memory shadow stack, whose pointer is
//! the module's one global, and the host engine's own. When the host's runs out, V8
//! throws a `RangeError` straight through every wasm frame of the call — no destructor
//! runs, the shadow-stack pointer is not moved back, and whatever the engine or its
//! allocator was midway through is left as it was. The instance that ran the statement
//! stays borrowed (wasm-bindgen's `recursive use of an object` on its next call), and the
//! *module* is left with a shadow stack short by that call's frames: measured under
//! `--no-liftoff` before the depth limit covered every operator chain, about six such
//! errors walked it below zero, after which every new `PgletBtree` — on a one-line
//! `CREATE TABLE` — trapped `memory access out of bounds`.
//!
//! Putting the pointer back from the JS glue would not make the module sound again: the
//! heap and the engine's thread-local state are as the unwound call left them. So the
//! module notices instead. Every export that runs the engine enters a [`ModuleCall`],
//! and one that finds the previous call never left refuses — then and on every later
//! call, on every instance — with a named error, rather than running on memory it cannot
//! trust. The depth limit (`pglet_sql::depth`'s wasm `DEFAULT_MAX_DEPTH`) is what keeps
//! a statement from getting there; this is what makes a host exception loud if one
//! does.
//!
//! What can unwind a call is what the module cannot catch: a `RangeError` from the JS
//! engine's own stack, a trap (the out-of-memory abort among them), and an exception
//! from JS the call reaches through an import that does not catch. The page-controlled
//! JS a call reaches is of two kinds, and both go through imports that **do** catch, so
//! an exception in either is the statement's error rather than an unwind:
//!
//! - what a statement runs — a bound parameter's getters and `toJSON`, and the copy out of a
//!   bound byte view (`crate::param`),
//!   and `Date.now` / `Math.random`, globals a page can replace (`pglet_sql::host`);
//! - what **building the result** runs, after the statement — the prototypes the result's
//!   objects and arrays inherit from, whose `Object.prototype` and `Array.prototype`
//!   setters a page owns; each store the module makes itself is caught, and none is dropped
//!   silently (`crate::result`'s `Store`): a data key is a `Reflect.defineProperty`; an array
//!   element a `Reflect.set` at the array's own length, checked to have moved that length — or
//!   a define, when a page put an index on the array's prototype chain — so no
//!   `Array.prototype.push` of the module's is reached; and a `bytea` value's bytes go in
//!   through a caught `Uint8Array.prototype.set`. A field descriptor is no store at all: it is
//!   one object literal, whose keys are defined and reach no page code. A store that answers
//!   `false`, or that left the array's `length` where it was, is the statement's error; page
//!   code on the prototype chain that moves `length` itself, or stores its own value, is not
//!   detected (`crate::result`'s `Store`).
//!
//! What is left is not the module's to catch: the JS intrinsics the **generated glue**
//! runs. It grows its own object heap with `heap.push`, reads its strings with
//! `TextDecoder.prototype.decode` and writes a non-ASCII string parameter with
//! `TextEncoder.prototype.encodeInto`, so a page that replaces any of them unwinds a call from
//! inside the glue — measured for all three, and the module then refuses every later call, which
//! is this module's loud answer rather than a wrong one
//! (`tests-web/integration/host_exceptions.ts`). A page that replaces the globals the
//! glue reads (`Object`, `Array`, `Reflect`) is past anything a catch of the module's
//! would make sound.
//!
//! **A call made while another is still running is refused too, with its own error**
//! ([`REENTERED`]): JS a statement called out to — a replaced `Date.now`, a parameter's
//! getter — can call back into the module, on any instance, and the engine's
//! thread-local state (the session a statement's stored expressions resolve against,
//! among them) belongs to the call already running. It is told apart from a call that
//! never returned by whether the running one is out in the host now
//! (`pglet_sql::host::in_call`). Only an unwind that lands inside that window — a
//! `RangeError` at the very edge of a call out, or a replaced glue intrinsic reached while the
//! call out's arguments are built (measured: `Uint8Array.prototype.subarray`) — would read as
//! the second kind.
//!
//! The `QueryResult` getters and the generated `free` functions enter no guard: they read
//! a result the engine already built, or drop one, and run no statement — all but the
//! persistent database's, whose drop rolls back a block left open and may save. Its close
//! takes a call of its own, and where that is refused the drop writes nothing
//! (`crate::persistent`'s `Drop for PgletBtreePersistent`).

use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::JsValue;

/// Set while an export is running the engine; left set by a call a host exception
/// unwound.
static IN_CALL: AtomicBool = AtomicBool::new(false);

/// One export's run of the engine, ended when dropped.
#[derive(Debug)]
#[must_use = "the call lasts as long as the guard"]
pub struct ModuleCall(());

impl ModuleCall {
    /// Enter a call.
    ///
    /// # Errors
    ///
    /// [`REENTERED`] when another call is running and out in the host now, else
    /// [`MODULE_POISONED`] when an earlier call never returned.
    pub fn try_enter() -> Result<Self, &'static str> {
        if IN_CALL.swap(true, Ordering::Relaxed) {
            // Left set either way: the running call, or the one that never returned,
            // still holds it.
            return Err(if pglet_sql::host::in_call() {
                REENTERED
            } else {
                MODULE_POISONED
            });
        }
        Ok(Self(()))
    }

    /// [`Self::try_enter`], its refusal as the JS error the export throws: an `Error`
    /// named `PgletModulePoisoned` or `PgletReentrantCall`, with no SQLSTATE `code` — no
    /// statement failed.
    ///
    /// # Errors
    ///
    /// As [`Self::try_enter`].
    pub fn enter() -> Result<Self, JsValue> {
        Self::try_enter().map_err(|message| {
            let error = js_sys::Error::new(message);
            error.set_name(if message == REENTERED {
                "PgletReentrantCall"
            } else {
                "PgletModulePoisoned"
            });
            error.into()
        })
    }
}

impl Drop for ModuleCall {
    fn drop(&mut self) {
        IN_CALL.store(false, Ordering::Relaxed);
    }
}

/// Held by every test that reads or sets the module's one static, so no two run at once,
/// each starting from a module no call holds.
#[cfg(test)]
pub fn exclusive_module() -> std::sync::MutexGuard<'static, ()> {
    static MODULE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = MODULE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset_module();
    guard
}

/// The module as no call has held it — for a test holding [`exclusive_module`].
#[cfg(test)]
pub fn reset_module() {
    IN_CALL.store(false, Ordering::Relaxed);
}

/// Why every call is refused once one never returned.
pub const MODULE_POISONED: &str = "pglet: an earlier call into this wasm module never returned — something the \
     module cannot catch unwound it mid-statement (a RangeError from the JS engine's own stack, a trap such as \
     running out of memory, or an exception from JS the call reached), so this module's memory can no longer \
     be trusted. Restart the process, or the Worker this module runs in: importing it again in the same one \
     answers the same instance.";

/// Why a call made from inside a running one is refused.
pub const REENTERED: &str = "pglet: a call into this wasm module was made while another call into it is still \
     running — from JS that call reached (a replaced Date.now or Math.random, a bound parameter's getter or \
     toJSON). The module runs one call at a time; make the call after the running one returns.";

#[cfg(test)]
mod tests {
    use super::{MODULE_POISONED, ModuleCall, REENTERED};

    // The guard is the module's one static, so a test that reads or sets it holds
    // `exclusive_module` — a second one on another thread would see this one's calls. A
    // call made from inside a running one — while
    // it is out in the host — is refused as re-entrant, not as a call that never
    // returned, and leaves the running one holding the module; a call that returns
    // leaves the module usable; and one that never does — its guard forgotten, as an
    // unwind through the wasm frames forgets it — refuses every call after it.
    #[test]
    fn a_reentrant_call_and_a_call_that_never_returned_are_refused_apart() {
        let _module = super::exclusive_module();
        let outer = ModuleCall::try_enter().expect("outer");
        let inner = pglet_sql::host::call(ModuleCall::try_enter);
        assert_eq!(inner.err(), Some(REENTERED));
        // Not out in the host, the same state is a call that never returned.
        assert_eq!(ModuleCall::try_enter().err(), Some(MODULE_POISONED));
        drop(outer);
        drop(ModuleCall::try_enter().expect("usable once the running call returns"));
        drop(ModuleCall::try_enter().expect("a returned call leaves it usable"));
        std::mem::forget(ModuleCall::try_enter().expect("third"));
        for _ in 0..3 {
            assert_eq!(ModuleCall::try_enter().err(), Some(MODULE_POISONED));
        }
    }
}
