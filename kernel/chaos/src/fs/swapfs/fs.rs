use crate::prelude::*;
use crate::*;

pub struct SwapFs {
    pub(crate) disk: Arc<dyn BlockDevice>,
    pub(crate) sb: RwLock<SwapFsSuperBlockDisk>,
    pub(crate) alloc: Mutex<()>,
    pub(crate) bitmap: Bitmap,
}

impl SwapFs {
    pub fn format(
        disk: Arc<dyn BlockDevice>,
        total_blocks: u64,
        max_files: usize,
    ) -> Result<Arc<Self>, &'static str> {
        validate_format_args(disk.as_ref(), total_blocks, max_files)?;
        let bitmap_block_count = bitmap_block_count(total_blocks)?;
        let meta_block_count = metadata_block_count(max_files)? as u64;
        let data_start_block = 1 + bitmap_block_count + meta_block_count;
        if data_start_block >= total_blocks {
            return Err("enospc");
        }
        if max_files > u32::MAX as usize {
            return Err("einval");
        }

        let sb = SwapFsSuperBlockDisk::new(
            total_blocks,
            bitmap_block_count,
            meta_block_count,
            max_files as u32,
        );
        sb.validate()?;

        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        sb.encode_into(&mut block)?;
        disk.write_block(0, &block)?;

        let zero_block = [0u8; SWAPFS_BLOCK_SIZE];
        for block_id in 1..data_start_block {
            disk.write_block(block_id, &zero_block)?;
        }
        disk.flush()?;

        let bitmap = bitmap_from_superblock(&sb)?;
        Ok(Arc::new(Self {
            disk,
            sb: RwLock::new(sb),
            alloc: Mutex::new(()),
            bitmap,
        }))
    }

    pub fn mount(disk: Arc<dyn BlockDevice>) -> Result<Arc<Self>, &'static str> {
        if disk.block_size() != SWAPFS_BLOCK_SIZE {
            return Err("einval");
        }

        let mut block = [0u8; SWAPFS_BLOCK_SIZE];
        disk.read_block(0, &mut block)?;
        let sb = SwapFsSuperBlockDisk::decode_from(&block)?;
        sb.validate()?;
        validate_mounted_superblock(disk.as_ref(), &sb)?;

        let bitmap = bitmap_from_superblock(&sb)?;
        Ok(Arc::new(Self {
            disk,
            sb: RwLock::new(sb),
            alloc: Mutex::new(()),
            bitmap,
        }))
    }

    pub fn mount_or_format(
        disk: Arc<dyn BlockDevice>,
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

    pub fn alloc_blocks(&self, block_count: u64) -> Result<u64, &'static str> {
        if block_count == 0 {
            return Ok(0);
        }
        let _alloc = self.alloc.lock().unwrap();
        self.bitmap.alloc_blocks(block_count, self.disk.as_ref())
    }
}

fn validate_format_args(
    disk: &dyn BlockDevice,
    total_blocks: u64,
    max_files: usize,
) -> Result<(), &'static str> {
    if disk.block_size() != SWAPFS_BLOCK_SIZE {
        return Err("einval");
    }
    if total_blocks != disk.block_count() {
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

fn validate_mounted_superblock(
    disk: &dyn BlockDevice,
    sb: &SwapFsSuperBlockDisk,
) -> Result<(), &'static str> {
    if sb.total_blocks != disk.block_count() {
        return Err("einval");
    }
    let max_files = sb.max_files as usize;
    let bitmap_capacity = sb
        .bitmap_block_count
        .checked_mul(SWAPFS_BITMAP_BITS_PER_BLOCK)
        .ok_or("einval")?;
    if bitmap_capacity < sb.total_blocks {
        return Err("einval");
    }
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

fn bitmap_from_superblock(sb: &SwapFsSuperBlockDisk) -> Result<Bitmap, &'static str> {
    let can_alloc_end = sb.total_blocks.checked_sub(1).ok_or("einval")?;
    Ok(Bitmap::new(
        sb.bitmap_start_block,
        sb.bitmap_block_count,
        sb.data_start_block,
        can_alloc_end,
    ))
}

fn metadata_block_count(max_files: usize) -> Result<usize, &'static str> {
    let rounded = max_files
        .checked_add(SWAPFS_META_PER_BLOCK - 1)
        .ok_or("einval")?;
    Ok(rounded / SWAPFS_META_PER_BLOCK)
}
