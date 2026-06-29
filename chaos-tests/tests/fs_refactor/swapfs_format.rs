use chaos_tests::*;
use std::sync::Arc;

#[test]
fn swapfs_format_writes_superblock_and_zero_metadata_blocks() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk.clone(), 32, 8).unwrap();

    assert_eq!(fs.max_files(), 8);
    assert_eq!(fs.next_free_block(), 3);

    let mut block = [0u8; SWAPFS_BLOCK_SIZE];
    assert_eq!(disk.read_block(0, &mut block), Ok(()));
    let sb = SwapFsSuperBlockDisk::decode_from(&block).unwrap();
    assert_eq!(sb.validate(), Ok(()));
    assert_eq!(sb.meta_block_count, 2);
    assert_eq!(sb.data_start_block, 3);
    assert_eq!(sb.next_free_block, 3);

    for block_id in 1..3 {
        let mut meta_block = [0xFFu8; SWAPFS_BLOCK_SIZE];
        assert_eq!(disk.read_block(block_id, &mut meta_block), Ok(()));
        assert!(meta_block.iter().all(|&b| b == 0));
    }
}

#[test]
fn swapfs_mount_rejects_unformatted_disk() {
    let disk = Arc::new(Disk::new("empty", 32, SWAPFS_BLOCK_SIZE));

    assert!(matches!(SwapFs::mount(disk), Err("einval")));
}

#[test]
fn swapfs_mount_loads_existing_superblock_and_metadata() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let _formatted = SwapFs::format(disk.clone(), 32, 4).unwrap();
    write_first_meta(&disk, SwapFsMetaDisk::new_used("keep", 2, 1, 4).unwrap());

    let mounted = SwapFs::mount(disk).unwrap();
    let sb = mounted.super_block();
    let meta = mounted.read_meta(0).unwrap();

    assert_eq!(sb.data_start_block, 2);
    assert_eq!(mounted.max_files(), 4);
    assert_eq!(mounted.next_free_block(), 2);
    assert!(meta.is_used());
    assert_eq!(meta.name_str(), Ok("keep"));
    assert_eq!(meta.start_block, 2);
    assert_eq!(meta.block_count, 1);
    assert_eq!(meta.size, 4);
}

#[test]
fn swapfs_mount_or_format_formats_empty_disk() {
    let disk = Arc::new(Disk::new("empty", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::mount_or_format(disk.clone(), 32, 4).unwrap();

    assert_eq!(fs.max_files(), 4);
    assert_eq!(fs.next_free_block(), 2);
    let mut block = [0u8; SWAPFS_BLOCK_SIZE];
    assert_eq!(disk.read_block(0, &mut block), Ok(()));
    let sb = SwapFsSuperBlockDisk::decode_from(&block).unwrap();
    assert_eq!(sb.magic, SWAPFS_MAGIC);
}

#[test]
fn swapfs_mount_or_format_preserves_existing_metadata() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let _formatted = SwapFs::format(disk.clone(), 32, 4).unwrap();
    write_first_meta(&disk, SwapFsMetaDisk::new_used("keep", 2, 1, 4).unwrap());

    let fs = SwapFs::mount_or_format(disk.clone(), 32, 4).unwrap();
    let meta = fs.read_meta(0).unwrap();

    assert!(meta.is_used());
    assert_eq!(meta.name_str(), Ok("keep"));

    let mut block = [0u8; SWAPFS_BLOCK_SIZE];
    assert_eq!(disk.read_block(1, &mut block), Ok(()));
    let raw = SwapFsMetaDisk::decode_from(&block[0..SWAPFS_META_DISK_SIZE]).unwrap();
    assert_eq!(raw.name_str(), Ok("keep"));
}

#[test]
fn swapfs_format_rejects_layout_without_data_blocks() {
    let disk = Arc::new(Disk::new("tiny", 2, SWAPFS_BLOCK_SIZE));

    assert!(matches!(SwapFs::format(disk, 2, 4), Err("enospc")));
}

#[test]
fn swapfs_read_write_meta_uses_metadata_index_location() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk.clone(), 32, 8).unwrap();
    let meta = SwapFsMetaDisk::new_used("slot4", 12, 1, 5).unwrap();

    assert_eq!(fs.write_meta(4, &meta), Ok(()));
    let loaded = fs.read_meta(4).unwrap();
    assert_eq!(loaded.name_str(), Ok("slot4"));
    assert_eq!(loaded.start_block, 12);

    let mut block_one = [0u8; SWAPFS_BLOCK_SIZE];
    let mut block_two = [0u8; SWAPFS_BLOCK_SIZE];
    disk.read_block(1, &mut block_one).unwrap();
    disk.read_block(2, &mut block_two).unwrap();
    assert!(block_one.iter().all(|&b| b == 0));
    assert_eq!(
        SwapFsMetaDisk::decode_from(&block_two[0..SWAPFS_META_DISK_SIZE])
            .unwrap()
            .name_str(),
        Ok("slot4")
    );
    assert_eq!(fs.read_meta(8), Err("einval"));
}

#[test]
fn swapfs_sync_super_persists_next_free_block() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk.clone(), 32, 4).unwrap();

    assert_eq!(fs.alloc_blocks(5), Ok(2));

    let remounted = SwapFs::mount(disk).unwrap();
    assert_eq!(remounted.next_free_block(), 7);
    assert_eq!(remounted.super_block().next_free_block, 7);
}

fn write_first_meta(disk: &Disk, meta: SwapFsMetaDisk) {
    let mut block = [0u8; SWAPFS_BLOCK_SIZE];
    disk.read_block(1, &mut block).unwrap();
    meta.encode_into(&mut block[0..SWAPFS_META_DISK_SIZE])
        .unwrap();
    disk.write_block(1, &block).unwrap();
}
