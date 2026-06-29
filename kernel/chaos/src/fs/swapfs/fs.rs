use crate::prelude::*;
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapFsAlloc {
    pub next_free_block: u64,
}

pub struct SwapFs {
    pub(crate) disk: Arc<Disk>,
    pub(crate) sb: RwLock<SwapFsSuperBlockDisk>,
    pub(crate) alloc: Mutex<SwapFsAlloc>,
}

impl SwapFs {
    pub fn format(
        disk: Arc<Disk>,
        total_blocks: u64,
        max_files: usize,
    ) -> Result<Arc<Self>, &'static str> {
        validate_format_args(&disk, total_blocks, max_files)?;
        let meta_block_count = metadata_block_count(max_files)? as u64;
        let data_start_block = 1 + meta_block_count;
        if data_start_block >= total_blocks {
            return Err("enospc");
        }
        if max_files > u32::MAX as usize {
            return Err("einval");
        }

        let sb = SwapFsSuperBlockDisk::new(
            total_blocks,
            meta_block_count,
            data_start_block,
            data_start_block,
            max_files as u32,
        );
        sb.validate()?;

        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        sb.encode_into(&mut block)?;
        disk.write_block(0, &block)?;

        let zero_block = [0u8; SWAPFS_BLOCK_SIZE];
        for block_id in 1..data_start_block as usize {
            disk.write_block(block_id, &zero_block)?;
        }
        disk.flush()?;

        Ok(Arc::new(Self {
            disk,
            sb: RwLock::new(sb),
            alloc: Mutex::new(SwapFsAlloc {
                next_free_block: data_start_block,
            }),
        }))
    }

    pub fn mount(disk: Arc<Disk>) -> Result<Arc<Self>, &'static str> {
        if disk.block_size() != SWAPFS_BLOCK_SIZE {
            return Err("einval");
        }

        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        disk.read_block(0, &mut block)?;
        let sb = SwapFsSuperBlockDisk::decode_from(&block)?;
        sb.validate()?;
        validate_mounted_superblock(&disk, &sb)?;

        let next_free_block = sb.next_free_block;
        Ok(Arc::new(Self {
            disk,
            sb: RwLock::new(sb),
            alloc: Mutex::new(SwapFsAlloc { next_free_block }),
        }))
    }

    pub fn mount_or_format(
        disk: Arc<Disk>,
        total_blocks: u64,
        max_files: usize,
    ) -> Result<Arc<Self>, &'static str> {
        match Self::mount(disk.clone()) {
            Ok(fs) => Ok(fs),
            Err(_) => Self::format(disk, total_blocks, max_files),
        }
    }

    pub fn super_block(&self) -> SwapFsSuperBlockDisk {
        self.sb.read().unwrap().clone()
    }

    pub fn max_files(&self) -> usize {
        self.sb.read().unwrap().max_files as usize
    }

    pub fn next_free_block(&self) -> u64 {
        self.alloc.lock().unwrap().next_free_block
    }

    pub fn alloc_blocks(&self, block_count: u64) -> Result<u64, &'static str> {
        if block_count == 0 {
            return Ok(self.next_free_block());
        }
        let total_blocks = self.sb.read().unwrap().total_blocks;
        let start_block = {
            let mut alloc = self.alloc.lock().unwrap();
            let start = alloc.next_free_block;
            let end = start.checked_add(block_count).ok_or("einval")?;
            if end > total_blocks {
                return Err("enospc");
            }
            alloc.next_free_block = end;
            start
        };
        self.sync_super()?;
        Ok(start_block)
    }

    pub fn sync_super(&self) -> Result<(), &'static str> {
        let mut sb = self.sb.write().unwrap();
        sb.next_free_block = self.alloc.lock().unwrap().next_free_block;
        sb.validate()?;
        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        sb.encode_into(&mut block)?;
        self.disk.write_block(0, &block)
    }

    pub fn read_meta(&self, meta_index: usize) -> Result<SwapFsMetaDisk, &'static str> {
        let (block_id, offset) = self.meta_location(meta_index)?;
        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        self.disk.read_block(block_id, &mut block)?;
        SwapFsMetaDisk::decode_from(&block[offset..offset + SWAPFS_META_DISK_SIZE])
    }

    pub fn write_meta(&self, meta_index: usize, meta: &SwapFsMetaDisk) -> Result<(), &'static str> {
        let (block_id, offset) = self.meta_location(meta_index)?;
        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        self.disk.read_block(block_id, &mut block)?;
        meta.encode_into(&mut block[offset..offset + SWAPFS_META_DISK_SIZE])?;
        self.disk.write_block(block_id, &block)
    }

    fn meta_location(&self, meta_index: usize) -> Result<(usize, usize), &'static str> {
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
        Ok((block_id as usize, slot * SWAPFS_META_DISK_SIZE))
    }
}

fn validate_format_args(
    disk: &Disk,
    total_blocks: u64,
    max_files: usize,
) -> Result<(), &'static str> {
    if disk.block_size() != SWAPFS_BLOCK_SIZE {
        return Err("einval");
    }
    if total_blocks != disk.block_count() as u64 {
        return Err("einval");
    }
    if total_blocks == 0 {
        return Err("einval");
    }
    if max_files == 0 {
        return Err("einval");
    }
    Ok(())
}

fn validate_mounted_superblock(disk: &Disk, sb: &SwapFsSuperBlockDisk) -> Result<(), &'static str> {
    if sb.total_blocks != disk.block_count() as u64 {
        return Err("einval");
    }
    let max_files = sb.max_files as usize;
    let meta_capacity = sb
        .meta_block_count
        .checked_mul(SWAPFS_META_PER_BLOCK as u64)
        .ok_or("einval")?;
    if max_files as u64 > meta_capacity {
        return Err("einval");
    }
    if sb.data_start_block > sb.total_blocks {
        return Err("einval");
    }
    Ok(())
}

fn metadata_block_count(max_files: usize) -> Result<usize, &'static str> {
    let rounded = max_files
        .checked_add(SWAPFS_META_PER_BLOCK - 1)
        .ok_or("einval")?;
    Ok(rounded / SWAPFS_META_PER_BLOCK)
}
