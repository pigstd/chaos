use crate::prelude::*;
use crate::*;

impl SwapFs {
    pub fn metadata_len(&self, meta_index: usize) -> Result<usize, &'static str> {
        let meta = self.read_meta(meta_index)?;
        self.validate_file_meta(&meta)?;
        meta_size_usize(&meta)
    }

    pub fn read_at(
        &self,
        meta_index: usize,
        off: usize,
        buf: &mut [u8],
    ) -> Result<usize, &'static str> {
        let meta = self.read_meta(meta_index)?;
        self.validate_file_meta(&meta)?;
        let size = meta_size_usize(&meta)?;
        if off >= size || buf.is_empty() {
            return Ok(0);
        }
        let len = min(buf.len(), size - off);
        self.read_from_blocks(meta.start_block, meta.block_count, off, &mut buf[..len])?;
        Ok(len)
    }

    pub fn write_at(
        &self,
        meta_index: usize,
        off: usize,
        buf: &[u8],
    ) -> Result<usize, &'static str> {
        let mut meta = self.read_meta(meta_index)?;
        self.validate_file_meta(&meta)?;
        if buf.is_empty() {
            return Ok(0);
        }

        let end = off.checked_add(buf.len()).ok_or("einval")?;
        let old_size = meta_size_usize(&meta)?;
        self.ensure_capacity(&mut meta, end)?;
        if off > old_size {
            self.zero_range(&meta, old_size, off - old_size)?;
        }
        self.write_to_blocks(meta.start_block, meta.block_count, off, buf)?;
        if end > old_size {
            meta.size = end as u64;
            self.write_meta(meta_index, &meta)?;
        }
        Ok(buf.len())
    }

    pub fn set_len(&self, meta_index: usize, len: usize) -> Result<(), &'static str> {
        let mut meta = self.read_meta(meta_index)?;
        self.validate_file_meta(&meta)?;
        let old_size = meta_size_usize(&meta)?;
        if len == old_size {
            return Ok(());
        }

        self.ensure_capacity(&mut meta, len)?;
        if len > old_size {
            self.zero_range(&meta, old_size, len - old_size)?;
        }
        meta.size = len as u64;
        self.write_meta(meta_index, &meta)
    }

    fn validate_file_meta(&self, meta: &SwapFsMetaDisk) -> Result<(), &'static str> {
        if !meta.is_used() {
            return Err("enoent");
        }
        let size = meta_size_usize(meta)?;
        let capacity = block_capacity_bytes(meta.block_count)?;
        if size > capacity {
            return Err("einval");
        }
        if meta.block_count == 0 {
            return Ok(());
        }

        let sb = self.sb.read().unwrap();
        if meta.start_block < sb.data_start_block {
            return Err("einval");
        }
        let end_block = meta
            .start_block
            .checked_add(meta.block_count)
            .ok_or("einval")?;
        if end_block > sb.total_blocks {
            return Err("einval");
        }
        Ok(())
    }

    fn ensure_capacity(
        &self,
        meta: &mut SwapFsMetaDisk,
        desired_size: usize,
    ) -> Result<(), &'static str> {
        let capacity = block_capacity_bytes(meta.block_count)?;
        if desired_size <= capacity {
            return Ok(());
        }

        let old_size = meta_size_usize(meta)?;
        let required_block_count = blocks_for_len(desired_size)?;
        let mut new_block_count = growth_block_count(meta.block_count, required_block_count)?;
        let new_start_block = match self.alloc_blocks(new_block_count) {
            Ok(start_block) => start_block,
            Err("enospc") if new_block_count != required_block_count => {
                new_block_count = required_block_count;
                self.alloc_blocks(new_block_count)?
            }
            Err(e) => return Err(e),
        };
        self.zero_blocks(new_start_block, new_block_count)?;
        if old_size > 0 {
            self.copy_between_blocks(
                meta.start_block,
                meta.block_count,
                new_start_block,
                new_block_count,
                old_size,
            )?;
        }

        meta.start_block = new_start_block;
        meta.block_count = new_block_count;
        Ok(())
    }

    fn copy_between_blocks(
        &self,
        src_start_block: u64,
        src_block_count: u64,
        dst_start_block: u64,
        dst_block_count: u64,
        len: usize,
    ) -> Result<(), &'static str> {
        let mut copied = 0;
        let mut scratch = [0u8; SWAPFS_BLOCK_SIZE];
        while copied < len {
            let chunk_len = min(SWAPFS_BLOCK_SIZE, len - copied);
            self.read_from_blocks(
                src_start_block,
                src_block_count,
                copied,
                &mut scratch[..chunk_len],
            )?;
            self.write_to_blocks(
                dst_start_block,
                dst_block_count,
                copied,
                &scratch[..chunk_len],
            )?;
            copied += chunk_len;
        }
        Ok(())
    }

    fn read_from_blocks(
        &self,
        start_block: u64,
        block_count: u64,
        off: usize,
        out: &mut [u8],
    ) -> Result<(), &'static str> {
        validate_byte_range(block_count, off, out.len())?;
        let mut copied = 0;
        while copied < out.len() {
            let pos = off.checked_add(copied).ok_or("einval")?;
            let block_offset = pos / SWAPFS_BLOCK_SIZE;
            let within_block = pos % SWAPFS_BLOCK_SIZE;
            let chunk_len = min(out.len() - copied, SWAPFS_BLOCK_SIZE - within_block);
            let block_id = start_block
                .checked_add(block_offset as u64)
                .ok_or("einval")?;
            let mut block = [0u8; SWAPFS_BLOCK_SIZE];
            self.disk
                .read_block(block_id_to_usize(block_id)?, &mut block)?;
            out[copied..copied + chunk_len]
                .copy_from_slice(&block[within_block..within_block + chunk_len]);
            copied += chunk_len;
        }
        Ok(())
    }

    fn write_to_blocks(
        &self,
        start_block: u64,
        block_count: u64,
        off: usize,
        input: &[u8],
    ) -> Result<(), &'static str> {
        validate_byte_range(block_count, off, input.len())?;
        let mut copied = 0;
        while copied < input.len() {
            let pos = off.checked_add(copied).ok_or("einval")?;
            let block_offset = pos / SWAPFS_BLOCK_SIZE;
            let within_block = pos % SWAPFS_BLOCK_SIZE;
            let chunk_len = min(input.len() - copied, SWAPFS_BLOCK_SIZE - within_block);
            let block_id = start_block
                .checked_add(block_offset as u64)
                .ok_or("einval")?;
            let mut block = [0u8; SWAPFS_BLOCK_SIZE];
            self.disk
                .read_block(block_id_to_usize(block_id)?, &mut block)?;
            block[within_block..within_block + chunk_len]
                .copy_from_slice(&input[copied..copied + chunk_len]);
            self.disk
                .write_block(block_id_to_usize(block_id)?, &block)?;
            copied += chunk_len;
        }
        Ok(())
    }

    fn zero_blocks(&self, start_block: u64, block_count: u64) -> Result<(), &'static str> {
        let zero = [0u8; SWAPFS_BLOCK_SIZE];
        for rel in 0..block_count {
            let block_id = start_block.checked_add(rel).ok_or("einval")?;
            self.disk.write_block(block_id_to_usize(block_id)?, &zero)?;
        }
        Ok(())
    }

    fn zero_range(
        &self,
        meta: &SwapFsMetaDisk,
        off: usize,
        len: usize,
    ) -> Result<(), &'static str> {
        let mut zeroed = 0;
        let zero = [0u8; SWAPFS_BLOCK_SIZE];
        while zeroed < len {
            let chunk_len = min(SWAPFS_BLOCK_SIZE, len - zeroed);
            self.write_to_blocks(
                meta.start_block,
                meta.block_count,
                off.checked_add(zeroed).ok_or("einval")?,
                &zero[..chunk_len],
            )?;
            zeroed += chunk_len;
        }
        Ok(())
    }
}

fn meta_size_usize(meta: &SwapFsMetaDisk) -> Result<usize, &'static str> {
    if meta.size > usize::MAX as u64 {
        return Err("einval");
    }
    Ok(meta.size as usize)
}

fn block_capacity_bytes(block_count: u64) -> Result<usize, &'static str> {
    if block_count > (usize::MAX / SWAPFS_BLOCK_SIZE) as u64 {
        return Err("einval");
    }
    Ok(block_count as usize * SWAPFS_BLOCK_SIZE)
}

fn blocks_for_len(len: usize) -> Result<u64, &'static str> {
    if len == 0 {
        return Ok(0);
    }
    let rounded = len.checked_add(SWAPFS_BLOCK_SIZE - 1).ok_or("einval")?;
    Ok((rounded / SWAPFS_BLOCK_SIZE) as u64)
}

fn growth_block_count(current: u64, required: u64) -> Result<u64, &'static str> {
    if required == 0 {
        return Ok(0);
    }
    let doubled = current.checked_mul(2).unwrap_or(required);
    Ok(max(required, max(1, doubled)))
}

fn validate_byte_range(block_count: u64, off: usize, len: usize) -> Result<(), &'static str> {
    if len == 0 {
        return Ok(());
    }
    let end = off.checked_add(len).ok_or("einval")?;
    let capacity = block_capacity_bytes(block_count)?;
    if end > capacity {
        return Err("einval");
    }
    Ok(())
}

fn block_id_to_usize(block_id: u64) -> Result<usize, &'static str> {
    if block_id > usize::MAX as u64 {
        return Err("einval");
    }
    Ok(block_id as usize)
}
