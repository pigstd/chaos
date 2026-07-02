use crate::prelude::*;
use crate::BlockDevice;

pub struct Disk {
    pub ops: AtomicUsize,
    pub label: String,
    block_size: usize,
    blocks: u64,
    storage: Mutex<Vec<u8>>,
}

impl BlockDevice for Disk {
    fn block_size(&self) -> usize { self.block_size() }

    fn block_count(&self) -> u64 { self.block_count() }

    fn read_block(&self, block_id: u64, out: &mut [u8]) -> Result<(), &'static str> {
        Disk::read_block(self, block_id, out)
    }

    fn write_block(&self, block_id: u64, data: &[u8]) -> Result<(), &'static str> {
        Disk::write_block(self, block_id, data)
    }

    fn flush(&self) -> Result<(), &'static str> {
        Disk::flush(self)
    }
}

impl Disk {
    pub fn new(label: &str, blocks: u64, block_size: usize) -> Self {
        let len = usize::try_from(blocks)
            .ok()
            .and_then(|blocks| blocks.checked_mul(block_size))
            .unwrap_or(0);
        Self {
            ops: AtomicUsize::new(0),
            label: label.to_string(),
            block_size,
            blocks,
            storage: Mutex::new(vec![0u8; len]),
        }
    }

    pub fn block_size(&self) -> usize { self.block_size }

    pub fn block_count(&self) -> u64 { self.blocks }

    pub fn read_block(&self, block_id: u64, out: &mut [u8]) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        if out.len() != self.block_size { return Err("einval"); }
        let (start, end) = self.block_range(block_id)?;
        let storage = self.storage.lock().unwrap();
        if end > storage.len() { return Err("einval"); }
        out.copy_from_slice(&storage[start..end]);
        Ok(())
    }

    pub fn write_block(&self, block_id: u64, data: &[u8]) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        if data.len() != self.block_size { return Err("einval"); }
        let (start, end) = self.block_range(block_id)?;
        let mut storage = self.storage.lock().unwrap();
        if end > storage.len() { return Err("einval"); }
        storage[start..end].copy_from_slice(data);
        Ok(())
    }

    pub fn flush(&self) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn total_ops(&self) -> usize { self.ops.load(Ordering::SeqCst) }

    pub fn reset_ops(&self) { self.ops.store(0, Ordering::SeqCst); }

    fn block_range(&self, block_id: u64) -> Result<(usize, usize), &'static str> {
        if block_id >= self.blocks { return Err("einval"); }
        let block_id = usize::try_from(block_id).map_err(|_| "einval")?;
        let start = block_id.checked_mul(self.block_size).ok_or("einval")?;
        let end = start.checked_add(self.block_size).ok_or("einval")?;
        Ok((start, end))
    }
}
