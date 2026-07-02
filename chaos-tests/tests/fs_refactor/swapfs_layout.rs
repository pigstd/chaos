use chaos_tests::*;

#[test]
fn swapfs_superblock_round_trips_from_block_bytes() {
    let sb = SwapFsSuperBlockDisk::new(128, 1, 2, 16);
    let mut block = [0xFFu8; SWAPFS_BLOCK_SIZE];

    assert_eq!(sb.encode_into(&mut block), Ok(()));
    assert_eq!(&block[0..4], &SWAPFS_MAGIC.to_le_bytes());
    assert!(block[SWAPFS_SUPER_BLOCK_DISK_SIZE..]
        .iter()
        .all(|&b| b == 0));

    let decoded = SwapFsSuperBlockDisk::decode_from(&block).unwrap();
    assert_eq!(decoded, sb);
    assert_eq!(decoded.validate(), Ok(()));
}

#[test]
fn swapfs_meta_round_trips_fixed_name_bytes() {
    let meta = SwapFsMetaDisk::new_used("hello.txt", 10, 2, 17).unwrap();
    let mut bytes = [0xFFu8; SWAPFS_META_DISK_SIZE];

    assert_eq!(meta.encode_into(&mut bytes), Ok(()));
    assert_eq!(SWAPFS_META_DISK_SIZE, 128);
    assert_eq!(SWAPFS_META_PER_BLOCK, 4);
    assert_eq!(SWAPFS_META_NAME_OFFSET % 8, 0);
    assert_eq!(SWAPFS_META_START_BLOCK_OFFSET % 8, 0);
    assert_eq!(SWAPFS_META_BLOCK_COUNT_OFFSET % 8, 0);
    assert_eq!(SWAPFS_META_SIZE_OFFSET % 8, 0);
    assert!(bytes[1..SWAPFS_META_NAME_OFFSET].iter().all(|&b| b == 0));
    assert!(bytes[96..SWAPFS_META_DISK_SIZE].iter().all(|&b| b == 0));
    let decoded = SwapFsMetaDisk::decode_from(&bytes).unwrap();

    assert!(decoded.is_used());
    assert_eq!(decoded.name_str(), Ok("hello.txt"));
    assert_eq!(decoded.start_block, 10);
    assert_eq!(decoded.block_count, 2);
    assert_eq!(decoded.size, 17);
}

#[test]
fn swapfs_layout_rejects_short_buffers() {
    let sb = SwapFsSuperBlockDisk::new(128, 1, 2, 16);
    let mut short_sb = [0u8; SWAPFS_SUPER_BLOCK_DISK_SIZE - 1];
    assert_eq!(sb.encode_into(&mut short_sb), Err("einval"));
    assert_eq!(SwapFsSuperBlockDisk::decode_from(&short_sb), Err("einval"));

    let meta = SwapFsMetaDisk::unused();
    let mut short_meta = [0u8; SWAPFS_META_DISK_SIZE - 1];
    assert_eq!(meta.encode_into(&mut short_meta), Err("einval"));
    assert_eq!(SwapFsMetaDisk::decode_from(&short_meta), Err("einval"));
}

#[test]
fn swapfs_layout_rejects_invalid_names_and_used_flags() {
    assert_eq!(encode_name(""), Err("einval"));
    assert_eq!(encode_name("dir/file"), Err("einval"));
    assert_eq!(encode_name("nul\0name"), Err("einval"));
    assert_eq!(encode_name(&"a".repeat(SWAPFS_NAME_LEN + 1)), Err("einval"));

    let mut bytes = [0u8; SWAPFS_META_DISK_SIZE];
    bytes[0] = 2;
    assert_eq!(SwapFsMetaDisk::decode_from(&bytes), Err("einval"));
}
