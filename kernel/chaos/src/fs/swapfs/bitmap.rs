use crate::{BlockDevice, SWAPFS_BITMAP_BITS_PER_BLOCK, SWAPFS_BLOCK_SIZE};

pub struct Bitmap {
    start_block_id: u64,
    blocks: u64,
    can_alloc_start: u64,
    can_alloc_end: u64,
}

pub struct BitmapBlock {
    data: [u8; SWAPFS_BLOCK_SIZE],
}

impl BitmapBlock {
    pub fn read_from_disk(block_id: u64, disk: &dyn BlockDevice) -> Result<Self, &'static str> {
        let mut data = [0u8; SWAPFS_BLOCK_SIZE];
        disk.read_block(block_id, &mut data)?;
        Ok(BitmapBlock { data })
    }

    pub fn write_to_disk(&self, block_id: u64, disk: &dyn BlockDevice) -> Result<(), &'static str> {
        disk.write_block(block_id, &self.data)
    }

    pub fn bit_status(&self, bit_index: usize) -> bool {
        let byte_index = bit_index / 8;
        let bit_offset = bit_index % 8;
        (self.data[byte_index] >> bit_offset & 1) == 1
    }
    pub fn set_bit(&mut self, bit_index: usize) {
        let byte_index = bit_index / 8;
        let bit_offset = bit_index % 8;
        self.data[byte_index] |= 1 << bit_offset;
    }
    pub fn clear_bit(&mut self, bit_index: usize) {
        let byte_index = bit_index / 8;
        let bit_offset = bit_index % 8;
        self.data[byte_index] &= !(1 << bit_offset);
    }
}

impl Bitmap {
    pub fn new(start_block_id: u64, blocks: u64, can_alloc_start: u64, can_alloc_end: u64) -> Self {
        Bitmap {
            start_block_id,
            blocks,
            can_alloc_start,
            can_alloc_end,
        }
    }

    pub fn alloc_blocks(&self, block_count: u64, disk: &dyn BlockDevice) -> Result<u64, &'static str> {
        if block_count == 0 {
            return Ok(0);
        }
        let mut continous_free_cout = 0;
        for block_id in self.start_block_id..self.start_block_id + self.blocks {
            let mut bitmap_block = BitmapBlock::read_from_disk(block_id, disk)?;
            for idx in 0..SWAPFS_BLOCK_SIZE * 8 {
                let global_idx = idx as u64 + (block_id - self.start_block_id) * SWAPFS_BITMAP_BITS_PER_BLOCK;
                if global_idx < self.can_alloc_start || global_idx > self.can_alloc_end {
                    continous_free_cout = 0;
                    continue;
                }
                if !bitmap_block.bit_status(idx) {
                    continous_free_cout += 1;
                    if continous_free_cout == block_count {
                        let end_idx = global_idx;
                        let start_idx = end_idx - block_count + 1;
                        self.set_use(start_idx, end_idx, disk)?;
                        return Ok(start_idx);
                    }
                } else {
                    continous_free_cout = 0;
                }
            }
        }
        Err("enospc")
    }
    // 把 start_idx 到 end_idx 的块标记为已使用
    pub fn set_use(&self, start_idx: u64, end_idx: u64, disk: &dyn BlockDevice) -> Result<(), &'static str> {
        if start_idx > end_idx || start_idx < self.can_alloc_start || end_idx > self.can_alloc_end {
            return Err("einval");
        }
        let start_block_id = self.start_block_id + start_idx / SWAPFS_BITMAP_BITS_PER_BLOCK;
        let end_block_id = self.start_block_id + end_idx / SWAPFS_BITMAP_BITS_PER_BLOCK;
        for block_id in start_block_id..=end_block_id {
            let mut bitmap_block = BitmapBlock::read_from_disk(block_id, disk)?;
            (0..SWAPFS_BLOCK_SIZE * 8).filter(|idx| {
                let global_idx = *idx as u64 + (block_id - self.start_block_id) * SWAPFS_BITMAP_BITS_PER_BLOCK;
                global_idx >= start_idx && global_idx <= end_idx
            }).for_each(|idx| {
                bitmap_block.set_bit(idx);
            });
            bitmap_block.write_to_disk(block_id, disk)?;
        }
        Ok(())
    }
    // 把 start_idx 到 end_idx 的块标记为 free
    pub fn set_free(&self, start_idx: u64, end_idx: u64, disk: &dyn BlockDevice) -> Result<(), &'static str> {
        if start_idx > end_idx || start_idx < self.can_alloc_start || end_idx > self.can_alloc_end {
            return Err("einval");
        }
        let start_block_id = self.start_block_id + start_idx / SWAPFS_BITMAP_BITS_PER_BLOCK;
        let end_block_id = self.start_block_id + end_idx / SWAPFS_BITMAP_BITS_PER_BLOCK;
        for block_id in start_block_id..=end_block_id {
            let mut bitmap_block = BitmapBlock::read_from_disk(block_id, disk)?;
            (0..SWAPFS_BLOCK_SIZE * 8).filter(|idx| {
                let global_idx = *idx as u64 + (block_id - self.start_block_id) * SWAPFS_BITMAP_BITS_PER_BLOCK;
                global_idx >= start_idx && global_idx <= end_idx
            }).for_each(|idx| {
                bitmap_block.clear_bit(idx);
            });
            bitmap_block.write_to_disk(block_id, disk)?;
        }
        Ok(())
    }
    // 检查 start_idx 到 end_idx 的块是否都是 free
    pub fn is_free(&self, start_idx: u64, end_idx: u64, disk: &dyn BlockDevice) -> Result<bool, &'static str> {
        if start_idx > end_idx {
            return Ok(true);
        }
        if start_idx < self.can_alloc_start || end_idx > self.can_alloc_end {
            return Ok(false);
        }
        let start_block_id = self.start_block_id + start_idx / SWAPFS_BITMAP_BITS_PER_BLOCK;
        let end_block_id = self.start_block_id + end_idx / SWAPFS_BITMAP_BITS_PER_BLOCK;
        for block_id in start_block_id..=end_block_id {
            let bitmap_block = BitmapBlock::read_from_disk(block_id, disk)?;
            for idx in 0..SWAPFS_BLOCK_SIZE * 8 {
                let global_idx = idx as u64 + (block_id - self.start_block_id) * SWAPFS_BITMAP_BITS_PER_BLOCK;
                if global_idx >= start_idx && global_idx <= end_idx && bitmap_block.bit_status(idx) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
