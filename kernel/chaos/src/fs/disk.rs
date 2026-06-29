use crate::prelude::*;

pub struct Disk {
    pub ops: AtomicUsize,
    pub label: String,
    block_size: usize,
    blocks: usize,
    storage: Mutex<Vec<u8>>,
}

impl Disk {
    pub fn new(label: &str, blocks: usize, block_size: usize) -> Self {
        let len = blocks.checked_mul(block_size).unwrap_or(0);
        Self {
            ops: AtomicUsize::new(0),
            label: label.to_string(),
            block_size,
            blocks,
            storage: Mutex::new(vec![0u8; len]),
        }
    }

    pub fn block_size(&self) -> usize { self.block_size }

    pub fn block_count(&self) -> usize { self.blocks }

    pub fn read_block(&self, block_id: usize, out: &mut [u8]) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        if out.len() != self.block_size { return Err("einval"); }
        let (start, end) = self.block_range(block_id)?;
        let storage = self.storage.lock().unwrap();
        out.copy_from_slice(&storage[start..end]);
        Ok(())
    }

    pub fn write_block(&self, block_id: usize, data: &[u8]) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        if data.len() != self.block_size { return Err("einval"); }
        let (start, end) = self.block_range(block_id)?;
        let mut storage = self.storage.lock().unwrap();
        storage[start..end].copy_from_slice(data);
        Ok(())
    }

    pub fn flush(&self) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn total_ops(&self) -> usize { self.ops.load(Ordering::SeqCst) }

    pub fn reset_ops(&self) { self.ops.store(0, Ordering::SeqCst); }

    fn block_range(&self, block_id: usize) -> Result<(usize, usize), &'static str> {
        if block_id >= self.blocks { return Err("einval"); }
        let start = block_id.checked_mul(self.block_size).ok_or("einval")?;
        let end = start.checked_add(self.block_size).ok_or("einval")?;
        Ok((start, end))
    }
}
