use chaos_tests::*;
use std::sync::Arc;

#[test]
fn swapfs_create_allocates_metadata_slot_and_data_blocks() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk.clone(), 32, 4).unwrap();

    let index = fs.create("/alpha", 1).unwrap();
    let meta = fs.read_meta(index).unwrap();

    assert_eq!(index, 0);
    assert!(meta.is_used());
    assert_eq!(meta.name_str(), Ok("alpha"));
    assert_eq!(meta.start_block, 2);
    assert_eq!(meta.block_count, 1);
    assert_eq!(meta.size, 0);
    assert_eq!(fs.next_free_block(), 3);

    let remounted = SwapFs::mount(disk).unwrap();
    assert_eq!(remounted.next_free_block(), 3);
    assert_eq!(remounted.open("alpha"), Ok(0));
}

#[test]
fn swapfs_open_and_metadata_len_use_disk_metadata() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk.clone(), 32, 4).unwrap();

    let index = fs.create("alpha", 1).unwrap();
    let mut meta = fs.read_meta(index).unwrap();
    meta.size = 17;
    fs.write_meta(index, &meta).unwrap();

    assert_eq!(fs.open("/alpha"), Ok(index));
    assert_eq!(fs.find_meta_by_name("alpha"), Ok(index));
    assert_eq!(fs.metadata_len(index), Ok(17));
}

#[test]
fn swapfs_open_or_create_does_not_reallocate_existing_file() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();

    let first = fs.create("alpha", 1).unwrap();
    assert_eq!(fs.next_free_block(), 3);
    let second = fs.open_or_create("/alpha", true, 8).unwrap();

    assert_eq!(second, first);
    assert_eq!(fs.next_free_block(), 3);
}

#[test]
fn swapfs_create_reuses_free_metadata_slot_but_not_old_blocks() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();

    assert_eq!(fs.create("first", 1), Ok(0));
    assert_eq!(fs.create("second", 1), Ok(1));
    assert_eq!(fs.next_free_block(), 4);

    fs.write_meta(0, &SwapFsMetaDisk::unused()).unwrap();
    assert_eq!(fs.find_free_meta(), Ok(0));

    let reused = fs.create("third", 1).unwrap();
    let meta = fs.read_meta(reused).unwrap();

    assert_eq!(reused, 0);
    assert_eq!(meta.name_str(), Ok("third"));
    assert_eq!(meta.start_block, 4);
    assert_eq!(fs.next_free_block(), 5);
}

#[test]
fn swapfs_metadata_operations_report_expected_errors() {
    let disk = Arc::new(Disk::new("swap0", 8, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 8, 2).unwrap();

    assert_eq!(fs.open("missing"), Err("enoent"));
    assert_eq!(fs.open_or_create("missing", false, 1), Err("enoent"));
    assert_eq!(fs.create("", 1), Err("einval"));
    assert_eq!(fs.create("/dir/file", 1), Err("einval"));
    assert_eq!(fs.metadata_len(0), Err("enoent"));

    assert_eq!(fs.create("a", 1), Ok(0));
    assert_eq!(fs.create("a", 1), Err("eexist"));
    assert_eq!(fs.create("b", 1), Ok(1));
    assert_eq!(fs.create("c", 1), Err("enospc"));
}

#[test]
fn swapfs_create_reports_enospc_when_data_blocks_are_exhausted() {
    let disk = Arc::new(Disk::new("swap0", 4, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 4, 4).unwrap();

    assert_eq!(fs.create("too-big", 3), Err("enospc"));
}
