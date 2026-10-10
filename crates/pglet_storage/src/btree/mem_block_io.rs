//! A shared in-memory [`BlockIo`] test double for `pglet_storage`'s own unit tests
//! (`block_io` and `double_meta` both use this one copy).
//!
//! `pglet_storage` can't borrow `pglet_testing`'s `MemBlockIo` here: a crate's
//! in-crate `#[cfg(test)]` build is a distinct instance, so a double that impls
//! `BlockIo` from the *dependency* copy of `pglet_storage` wouldn't satisfy this
//! crate's `BlockIo`. (Integration tests in `pglet` / `pglet_sql` unify normally
//! and do use `pglet_testing`.)

use std::sync::Mutex;

use super::block_io::BlockIo;

/// An in-memory, drop-surviving `BlockIo`: a growable byte buffer behind a mutex.
pub struct MemBlockIo {
    data: Mutex<Vec<u8>>,
}

impl MemBlockIo {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Vec::new()),
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            data: Mutex::new(bytes),
        }
    }
}

impl BlockIo for MemBlockIo {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let data = self.data.lock().unwrap();
        let start = offset as usize;
        if start >= data.len() {
            return Ok(0);
        }
        let end = (start + buf.len()).min(data.len());
        let n = end - start;
        buf[..n].copy_from_slice(&data[start..end]);
        Ok(n)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        let mut data = self.data.lock().unwrap();
        let end = offset as usize + buf.len();
        if data.len() < end {
            data.resize(end, 0);
        }
        data[offset as usize..end].copy_from_slice(buf);
        Ok(())
    }

    fn sync(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.data.lock().unwrap().len() as u64)
    }

    fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.data.lock().unwrap().resize(size as usize, 0);
        Ok(())
    }
}
