//! OPFS-backed `BlockIo` for the single-file double-meta store in the browser.
//!
//! This is the browser analogue of `pglet_storage`'s native `FileBlockIo`: it
//! implements `pglet_storage::btree::BlockIo` (positioned reads/writes, fsync,
//! length management) over a single OPFS `FileSystemSyncAccessHandle`. The
//! `DoubleMetaStore` works over any `BlockIo` (it holds a `Box<dyn BlockIo>`), so
//! the same store — and the same crash-safe two-meta-slot commit protocol — runs
//! natively over a file and in the browser over OPFS, with no separate page-store fork.
//!
//! # Web Worker requirement
//!
//! The sync file access API (`FileSystemSyncAccessHandle`) is only available in
//! Web Workers, not on the main thread.

// The OPFS async fns await JS handles (`JsFuture`, `FileSystemDirectoryHandle`, …)
// that are never `Send` — but WASM is single-threaded, so these futures never cross
// a thread boundary. Same justification as the audited `unsafe impl Send+Sync` below.
#![allow(clippy::future_not_send)]
// The OPFS sync handle isn't `Send`/`Sync`; the `unsafe impl`s below vouch for
// it under single-threaded WASM (see `SyncHandle`). This is the one audited
// `unsafe` in the codebase, and this module the one place the crate's
// `unsafe_code = "deny"` is lifted.
#![allow(unsafe_code)]

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemSyncAccessHandle};

use pglet_storage::btree::BlockIo;

/// Default data-file name in OPFS for the single-file double-meta store.
const DATA_FILENAME: &str = "data.pglet";

/// Wrapper around `FileSystemSyncAccessHandle` that implements `Send + Sync`.
///
/// # Safety
///
/// This is safe because WASM is single-threaded. The `Send + Sync` bounds are
/// only required by the `BlockIo: Send + Sync` trait signature, not for actual
/// thread safety — the handle never crosses a thread.
struct SyncHandle(FileSystemSyncAccessHandle);

// SAFETY: WASM is single-threaded, so Send + Sync are trivially satisfied. The lint correctly
// observes the wrapped JS handle isn't `Send` — that's exactly what this `unsafe impl` vouches for.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for SyncHandle {}
unsafe impl Sync for SyncHandle {}

/// Map a `JsValue` error to a `std::io::Error` with context.
// The `map_err` closures own their `JsValue` and pass it straight to formatting; taking it by
// value (vs `&JsValue`) keeps each call site a plain `|e| io_err("x", e)`.
#[allow(clippy::needless_pass_by_value)]
fn io_err(context: &str, e: JsValue) -> std::io::Error {
    std::io::Error::other(format!("{context}: {e:?}"))
}

/// Single-file OPFS-backed `BlockIo`.
///
/// Holds one OPFS sync access handle for the data file. Offsets are absolute
/// byte offsets from the start of the file, matching the native `FileBlockIo`.
pub struct OpfsBlockIo {
    handle: SyncHandle,
}

impl std::fmt::Debug for OpfsBlockIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpfsBlockIo").finish_non_exhaustive()
    }
}

impl OpfsBlockIo {
    /// Acquire the single data-file sync handle from OPFS, creating the file if
    /// absent.
    ///
    /// This is async because acquiring the OPFS directory and sync handle goes
    /// through promises; the resulting `BlockIo` is then fully synchronous.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` if OPFS is unavailable (e.g. not in a Web Worker) or
    /// the handle can't be created.
    pub async fn open() -> Result<Self, JsValue> {
        let root = get_opfs_root().await?;
        let handle = get_sync_handle(&root, DATA_FILENAME, true).await?;
        Ok(Self {
            handle: SyncHandle(handle),
        })
    }
}

impl BlockIo for OpfsBlockIo {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let buffer = Uint8Array::new_with_length(buf.len() as u32);
        let options = web_sys::FileSystemReadWriteOptions::new();
        options.set_at(offset as f64);

        let read = self
            .handle
            .0
            .read_with_buffer_source_and_options(&buffer, &options)
            .map_err(|e| io_err("read_at", e))?;

        let n = read as usize;
        buffer.slice(0, n as u32).copy_to(&mut buf[..n]);
        Ok(n)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        let buffer = Uint8Array::new_with_length(buf.len() as u32);
        buffer.copy_from(buf);

        let options = web_sys::FileSystemReadWriteOptions::new();
        options.set_at(offset as f64);

        let written = self
            .handle
            .0
            .write_with_buffer_source_and_options(&buffer, &options)
            .map_err(|e| io_err("write_at", e))?;

        if (written as usize) != buf.len() {
            return Err(std::io::Error::other(format!(
                "OPFS short write: expected {} bytes, wrote {written}",
                buf.len()
            )));
        }
        Ok(())
    }

    fn sync(&self) -> std::io::Result<()> {
        self.handle.0.flush().map_err(|e| io_err("sync", e))
    }

    fn len(&self) -> std::io::Result<u64> {
        let size = self.handle.0.get_size().map_err(|e| io_err("len", e))?;
        Ok(size as u64)
    }

    fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.handle
            .0
            .truncate_with_f64(size as f64)
            .map_err(|e| io_err("set_len", e))
    }
}

/// Get the OPFS root directory handle.
async fn get_opfs_root() -> Result<FileSystemDirectoryHandle, JsValue> {
    // Try worker context first (most common for persistence use case).
    let storage = if let Ok(scope) = js_sys::global().dyn_into::<web_sys::WorkerGlobalScope>() {
        scope.navigator().storage()
    } else if let Some(window) = web_sys::window() {
        window.navigator().storage()
    } else {
        return Err(JsValue::from_str("No window or worker global scope found"));
    };

    let promise = storage.get_directory();
    let result = JsFuture::from(promise).await?;
    Ok(result.unchecked_into())
}

/// Get a sync access handle for a file in OPFS.
async fn get_sync_handle(
    root: &FileSystemDirectoryHandle,
    filename: &str,
    create: bool,
) -> Result<FileSystemSyncAccessHandle, JsValue> {
    let options = web_sys::FileSystemGetFileOptions::new();
    options.set_create(create);

    let promise = root.get_file_handle_with_options(filename, &options);
    let file_handle: FileSystemFileHandle = JsFuture::from(promise).await?.unchecked_into();

    let promise = file_handle.create_sync_access_handle();
    let sync_handle: FileSystemSyncAccessHandle = JsFuture::from(promise).await?.unchecked_into();

    Ok(sync_handle)
}
