use crate::*;

impl SwapFs {
    pub fn read_meta(&self, meta_index: usize) -> Result<SwapFsMetaDisk, &'static str> {
        let _guard = self.op_lock.read_guard();
        self.read_meta_locked(meta_index)
    }

    pub(crate) fn read_meta_locked(&self, meta_index: usize) -> Result<SwapFsMetaDisk, &'static str> {
        let (block_id, offset) = self.meta_location(meta_index)?;
        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        self.disk.read_block(block_id, &mut block)?;
        SwapFsMetaDisk::decode_from(&block[offset..offset + SWAPFS_META_DISK_SIZE])
    }

    pub fn write_meta(&self, meta_index: usize, meta: &SwapFsMetaDisk) -> Result<(), &'static str> {
        let _guard = self.op_lock.write_guard();
        self.write_meta_locked(meta_index, meta)
    }

    pub(crate) fn write_meta_locked(&self, meta_index: usize, meta: &SwapFsMetaDisk) -> Result<(), &'static str> {
        let (block_id, offset) = self.meta_location(meta_index)?;
        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        self.disk.read_block(block_id, &mut block)?;
        meta.encode_into(&mut block[offset..offset + SWAPFS_META_DISK_SIZE])?;
        self.disk.write_block(block_id, &block)
    }

    pub fn find_meta_by_name(&self, name: &str) -> Result<usize, &'static str> {
        let _guard = self.op_lock.read_guard();
        self.find_meta_by_name_locked(name)
    }

    pub(crate) fn find_meta_by_name_locked(&self, name: &str) -> Result<usize, &'static str> {
        let normalized = normalize_name(name)?;
        for index in 0..self.max_files() {
            let meta = self.read_meta_locked(index)?;
            if meta.is_used() && meta.name_str()? == normalized {
                return Ok(index);
            }
        }
        Err("enoent")
    }

    pub fn find_free_meta(&self) -> Result<usize, &'static str> {
        let _guard = self.op_lock.read_guard();
        self.find_free_meta_locked()
    }

    pub(crate) fn find_free_meta_locked(&self) -> Result<usize, &'static str> {
        for index in 0..self.max_files() {
            let meta = self.read_meta_locked(index)?;
            if !meta.is_used() {
                return Ok(index);
            }
        }
        Err("enospc")
    }

    pub fn open(&self, name: &str) -> Result<usize, &'static str> {
        let _guard = self.op_lock.read_guard();
        self.find_meta_by_name_locked(name)
    }

    pub fn create(&self, name: &str, initial_blocks: u64) -> Result<usize, &'static str> {
        let _guard = self.op_lock.write_guard();
        self.create_locked(name, initial_blocks)
    }

    pub(crate) fn create_locked(&self, name: &str, initial_blocks: u64) -> Result<usize, &'static str> {
        let normalized = normalize_name(name)?;
        match self.find_meta_by_name_locked(&normalized) {
            Ok(_) => return Err("eexist"),
            Err("enoent") => {}
            Err(e) => return Err(e),
        }
        let index = self.find_free_meta_locked()?;
        let start_block = self.alloc_blocks_locked(initial_blocks)?;
        let meta = SwapFsMetaDisk::new_used(&normalized, start_block, initial_blocks, 0)?;
        self.write_meta_locked(index, &meta)?;
        Ok(index)
    }

    pub fn open_or_create(
        &self,
        name: &str,
        create: bool,
        initial_blocks: u64,
    ) -> Result<usize, &'static str> {
        if create {
            let _guard = self.op_lock.write_guard();
            self.open_or_create_locked(name, create, initial_blocks)
        } else {
            let _guard = self.op_lock.read_guard();
            self.open_or_create_locked(name, create, initial_blocks)
        }
    }

    pub(crate) fn open_or_create_locked(
        &self,
        name: &str,
        create: bool,
        initial_blocks: u64,
    ) -> Result<usize, &'static str> {
        match self.find_meta_by_name_locked(name) {
            Ok(index) => Ok(index),
            Err("enoent") if create => self.create_locked(name, initial_blocks),
            Err(e) => Err(e),
        }
    }

    fn meta_location(&self, meta_index: usize) -> Result<(u64, usize), &'static str> {
        let sb = self.sb.read().unwrap();
        if meta_index >= sb.max_files as usize {
            return Err("einval");
        }
        let block_offset = meta_index / SWAPFS_META_PER_BLOCK;
        let slot = meta_index % SWAPFS_META_PER_BLOCK;
        let block_id = sb
            .meta_start_block
            .checked_add(block_offset as u64)
            .ok_or("einval")?;
        if block_id >= sb.data_start_block {
            return Err("einval");
        }
        Ok((block_id, slot * SWAPFS_META_DISK_SIZE))
    }
}

fn normalize_name(path: &str) -> Result<String, &'static str> {
    let name = path.strip_prefix('/').unwrap_or(path);
    encode_name(name)?;
    Ok(name.to_string())
}
