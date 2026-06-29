use chaos_tests::*;
use std::sync::Arc;

#[test]
fn swapfs_write_then_read_round_trips_file_bytes() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();
    let index = fs.create("alpha", 1).unwrap();

    assert_eq!(fs.write_at(index, 0, b"hello"), Ok(5));
    assert_eq!(fs.metadata_len(index), Ok(5));

    let mut buf = [0xCCu8; 8];
    assert_eq!(fs.read_at(index, 0, &mut buf), Ok(5));
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(&buf[5..], &[0xCC, 0xCC, 0xCC]);

    let mut tail = [0u8; 8];
    assert_eq!(fs.read_at(index, 2, &mut tail), Ok(3));
    assert_eq!(&tail[..3], b"llo");
    assert_eq!(fs.read_at(index, 5, &mut tail), Ok(0));
}

#[test]
fn swapfs_write_across_block_boundary_moves_to_larger_extent() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();
    let index = fs.create("large", 1).unwrap();
    let data = patterned_bytes(SWAPFS_BLOCK_SIZE + 88);

    assert_eq!(fs.write_at(index, 0, &data), Ok(data.len()));

    let meta = fs.read_meta(index).unwrap();
    assert_eq!(meta.start_block, 3);
    assert_eq!(meta.block_count, 2);
    assert_eq!(meta.size, data.len() as u64);
    assert_eq!(fs.next_free_block(), 5);

    let mut out = vec![0u8; data.len()];
    assert_eq!(fs.read_at(index, 0, &mut out), Ok(data.len()));
    assert_eq!(out, data);
}

#[test]
fn swapfs_growth_uses_vector_like_extra_capacity() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();
    let index = fs.create("grow", 2).unwrap();

    assert_eq!(fs.write_at(index, 0, b"seed"), Ok(4));
    assert_eq!(fs.write_at(index, SWAPFS_BLOCK_SIZE * 2, b"x"), Ok(1));

    let meta = fs.read_meta(index).unwrap();
    assert_eq!(meta.start_block, 4);
    assert_eq!(meta.block_count, 4);
    assert_eq!(meta.size, (SWAPFS_BLOCK_SIZE * 2 + 1) as u64);
    assert_eq!(fs.next_free_block(), 8);

    let mut out = vec![0xAAu8; SWAPFS_BLOCK_SIZE * 2 + 1];
    let out_len = out.len();
    assert_eq!(fs.read_at(index, 0, &mut out), Ok(out_len));
    assert_eq!(&out[..4], b"seed");
    assert!(out[4..SWAPFS_BLOCK_SIZE * 2].iter().all(|&b| b == 0));
    assert_eq!(out[SWAPFS_BLOCK_SIZE * 2], b'x');
}

#[test]
fn swapfs_growth_falls_back_when_doubled_capacity_does_not_fit() {
    let disk = Arc::new(Disk::new("swap0", 7, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 7, 4).unwrap();
    let index = fs.create("tight", 2).unwrap();

    assert_eq!(fs.write_at(index, 0, b"seed"), Ok(4));
    assert_eq!(fs.write_at(index, SWAPFS_BLOCK_SIZE * 2, b"x"), Ok(1));

    let meta = fs.read_meta(index).unwrap();
    assert_eq!(meta.start_block, 4);
    assert_eq!(meta.block_count, 3);
    assert_eq!(meta.size, (SWAPFS_BLOCK_SIZE * 2 + 1) as u64);
    assert_eq!(fs.next_free_block(), 7);

    let mut out = vec![0u8; 4];
    assert_eq!(fs.read_at(index, 0, &mut out), Ok(4));
    assert_eq!(&out, b"seed");
}

#[test]
fn swapfs_sparse_write_preserves_old_data_and_zeroes_gap() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();
    let index = fs.create("sparse", 1).unwrap();

    assert_eq!(fs.write_at(index, 0, b"abc"), Ok(3));
    let tail_off = SWAPFS_BLOCK_SIZE + 10;
    assert_eq!(fs.write_at(index, tail_off, b"tail"), Ok(4));

    let file_len = tail_off + 4;
    let mut out = vec![0xAAu8; file_len];
    assert_eq!(fs.read_at(index, 0, &mut out), Ok(file_len));
    assert_eq!(&out[..3], b"abc");
    assert!(out[3..tail_off].iter().all(|&b| b == 0));
    assert_eq!(&out[tail_off..], b"tail");
}

#[test]
fn swapfs_set_len_grows_with_zeroes_and_shrinks_visible_size() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 32, 4).unwrap();
    let index = fs.create("sized", 0).unwrap();

    assert_eq!(fs.set_len(index, SWAPFS_BLOCK_SIZE + 3), Ok(()));
    assert_eq!(fs.metadata_len(index), Ok(SWAPFS_BLOCK_SIZE + 3));

    let mut grown = vec![0xDDu8; SWAPFS_BLOCK_SIZE + 3];
    assert_eq!(fs.read_at(index, 0, &mut grown), Ok(SWAPFS_BLOCK_SIZE + 3));
    assert!(grown.iter().all(|&b| b == 0));

    assert_eq!(fs.write_at(index, 10, b"abc"), Ok(3));
    assert_eq!(fs.set_len(index, 12), Ok(()));
    assert_eq!(fs.metadata_len(index), Ok(12));

    let mut shrunk = vec![0u8; 32];
    assert_eq!(fs.read_at(index, 0, &mut shrunk), Ok(12));
    assert_eq!(&shrunk[10..12], b"ab");
    assert_eq!(fs.read_at(index, 12, &mut shrunk), Ok(0));
}

#[test]
fn swapfs_written_data_survives_remount() {
    let disk = Arc::new(Disk::new("swap0", 32, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk.clone(), 32, 4).unwrap();
    let index = fs.create("keep", 1).unwrap();

    assert_eq!(fs.write_at(index, 7, b"persist"), Ok(7));

    let remounted = SwapFs::mount(disk).unwrap();
    let reopened = remounted.open("keep").unwrap();
    let mut out = vec![0xAAu8; 14];
    assert_eq!(remounted.read_at(reopened, 0, &mut out), Ok(14));
    assert!(out[..7].iter().all(|&b| b == 0));
    assert_eq!(&out[7..], b"persist");
}

#[test]
fn swapfs_data_operations_report_expected_errors() {
    let disk = Arc::new(Disk::new("swap0", 4, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 4, 4).unwrap();
    let index = fs.create("full", 1).unwrap();
    let too_large = vec![1u8; SWAPFS_BLOCK_SIZE + 1];

    assert_eq!(fs.read_at(1, 0, &mut [0u8; 1]), Err("enoent"));
    assert_eq!(fs.write_at(1, 0, b"x"), Err("enoent"));
    assert_eq!(fs.set_len(1, 1), Err("enoent"));

    assert_eq!(fs.write_at(index, 0, &too_large), Err("enospc"));
    assert_eq!(fs.metadata_len(index), Ok(0));
    assert_eq!(fs.next_free_block(), 3);
}

fn patterned_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}
