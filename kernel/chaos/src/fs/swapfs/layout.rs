pub const SWAPFS_BLOCK_SIZE: usize = 512;
pub const SWAPFS_NAME_LEN: usize = 64;
pub const SWAPFS_MAGIC: u32 = 0x5357_4150;
pub const SWAPFS_VERSION: u32 = 1;

pub const SWAPFS_SUPER_BLOCK_DISK_SIZE: usize = 56;
pub const SWAPFS_META_DISK_SIZE: usize = 128;
pub const SWAPFS_META_PER_BLOCK: usize = SWAPFS_BLOCK_SIZE / SWAPFS_META_DISK_SIZE;
pub const SWAPFS_META_NAME_OFFSET: usize = 8;
pub const SWAPFS_META_START_BLOCK_OFFSET: usize = 72;
pub const SWAPFS_META_BLOCK_COUNT_OFFSET: usize = 80;
pub const SWAPFS_META_SIZE_OFFSET: usize = 88;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapFsSuperBlockDisk {
    pub magic: u32,
    pub version: u32,
    pub block_size: u32,
    pub total_blocks: u64,
    pub meta_start_block: u64,
    pub meta_block_count: u64,
    pub data_start_block: u64,
    pub next_free_block: u64,
    pub max_files: u32,
}

impl SwapFsSuperBlockDisk {
    pub fn new(
        total_blocks: u64,
        meta_block_count: u64,
        data_start_block: u64,
        next_free_block: u64,
        max_files: u32,
    ) -> Self {
        Self {
            magic: SWAPFS_MAGIC,
            version: SWAPFS_VERSION,
            block_size: SWAPFS_BLOCK_SIZE as u32,
            total_blocks,
            meta_start_block: 1,
            meta_block_count,
            data_start_block,
            next_free_block,
            max_files,
        }
    }

    pub fn encode_into(&self, out: &mut [u8]) -> Result<(), &'static str> {
        if out.len() < SWAPFS_SUPER_BLOCK_DISK_SIZE {
            return Err("einval");
        }
        out.fill(0);
        write_u32(out, 0, self.magic);
        write_u32(out, 4, self.version);
        write_u32(out, 8, self.block_size);
        write_u64(out, 12, self.total_blocks);
        write_u64(out, 20, self.meta_start_block);
        write_u64(out, 28, self.meta_block_count);
        write_u64(out, 36, self.data_start_block);
        write_u64(out, 44, self.next_free_block);
        write_u32(out, 52, self.max_files);
        Ok(())
    }

    pub fn decode_from(input: &[u8]) -> Result<Self, &'static str> {
        if input.len() < SWAPFS_SUPER_BLOCK_DISK_SIZE {
            return Err("einval");
        }
        Ok(Self {
            magic: read_u32(input, 0),
            version: read_u32(input, 4),
            block_size: read_u32(input, 8),
            total_blocks: read_u64(input, 12),
            meta_start_block: read_u64(input, 20),
            meta_block_count: read_u64(input, 28),
            data_start_block: read_u64(input, 36),
            next_free_block: read_u64(input, 44),
            max_files: read_u32(input, 52),
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.magic != SWAPFS_MAGIC {
            return Err("einval");
        }
        if self.version != SWAPFS_VERSION {
            return Err("einval");
        }
        if self.block_size as usize != SWAPFS_BLOCK_SIZE {
            return Err("einval");
        }
        if self.meta_start_block != 1 {
            return Err("einval");
        }
        if self.meta_block_count == 0 {
            return Err("einval");
        }
        let expected_data_start = self
            .meta_start_block
            .checked_add(self.meta_block_count)
            .ok_or("einval")?;
        if self.data_start_block != expected_data_start {
            return Err("einval");
        }
        if self.next_free_block < self.data_start_block {
            return Err("einval");
        }
        if self.next_free_block > self.total_blocks {
            return Err("einval");
        }
        if self.max_files == 0 {
            return Err("einval");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapFsMetaDisk {
    pub used: u8,
    pub name: [u8; SWAPFS_NAME_LEN],
    pub start_block: u64,
    pub block_count: u64,
    pub size: u64,
}

impl SwapFsMetaDisk {
    pub fn unused() -> Self {
        Self {
            used: 0,
            name: [0; SWAPFS_NAME_LEN],
            start_block: 0,
            block_count: 0,
            size: 0,
        }
    }

    pub fn new_used(
        name: &str,
        start_block: u64,
        block_count: u64,
        size: u64,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            used: 1,
            name: encode_name(name)?,
            start_block,
            block_count,
            size,
        })
    }

    pub fn encode_into(&self, out: &mut [u8]) -> Result<(), &'static str> {
        if out.len() < SWAPFS_META_DISK_SIZE {
            return Err("einval");
        }
        if self.used > 1 {
            return Err("einval");
        }
        out[..SWAPFS_META_DISK_SIZE].fill(0);
        out[0] = self.used;
        out[SWAPFS_META_NAME_OFFSET..SWAPFS_META_NAME_OFFSET + SWAPFS_NAME_LEN]
            .copy_from_slice(&self.name);
        write_u64(out, SWAPFS_META_START_BLOCK_OFFSET, self.start_block);
        write_u64(out, SWAPFS_META_BLOCK_COUNT_OFFSET, self.block_count);
        write_u64(out, SWAPFS_META_SIZE_OFFSET, self.size);
        Ok(())
    }

    pub fn decode_from(input: &[u8]) -> Result<Self, &'static str> {
        if input.len() < SWAPFS_META_DISK_SIZE {
            return Err("einval");
        }
        let used = input[0];
        if used > 1 {
            return Err("einval");
        }
        let mut name = [0u8; SWAPFS_NAME_LEN];
        name.copy_from_slice(
            &input[SWAPFS_META_NAME_OFFSET..SWAPFS_META_NAME_OFFSET + SWAPFS_NAME_LEN],
        );
        Ok(Self {
            used,
            name,
            start_block: read_u64(input, SWAPFS_META_START_BLOCK_OFFSET),
            block_count: read_u64(input, SWAPFS_META_BLOCK_COUNT_OFFSET),
            size: read_u64(input, SWAPFS_META_SIZE_OFFSET),
        })
    }

    pub fn is_used(&self) -> bool {
        self.used != 0
    }

    pub fn name_str(&self) -> Result<&str, &'static str> {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(SWAPFS_NAME_LEN);
        std::str::from_utf8(&self.name[..len]).map_err(|_| "einval")
    }
}

pub fn encode_name(name: &str) -> Result<[u8; SWAPFS_NAME_LEN], &'static str> {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return Err("einval");
    }
    if bytes.len() > SWAPFS_NAME_LEN {
        return Err("einval");
    }
    if bytes.iter().any(|&b| b == 0 || b == b'/') {
        return Err("einval");
    }
    let mut out = [0u8; SWAPFS_NAME_LEN];
    out[..bytes.len()].copy_from_slice(bytes);
    Ok(out)
}

fn write_u32(out: &mut [u8], off: usize, value: u32) {
    out[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut [u8], off: usize, value: u64) {
    out[off..off + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(input: &[u8], off: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&input[off..off + 4]);
    u32::from_le_bytes(bytes)
}

fn read_u64(input: &[u8], off: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&input[off..off + 8]);
    u64::from_le_bytes(bytes)
}
