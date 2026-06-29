use chaos_tests::*;

#[test]
fn basic_disk_starts_zeroed() {
    let d = Disk::new("d0", 4, 512);
    let mut buf = [0xFFu8; 512];

    assert_eq!(d.block_size(), 512);
    assert_eq!(d.block_count(), 4);
    assert_eq!(d.read_block(0, &mut buf), Ok(()));
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn basic_disk_write_then_read_same_block() {
    let d = Disk::new("d0", 8, 512);
    let mut input = [0u8; 512];
    for (idx, b) in input.iter_mut().enumerate() {
        *b = (idx % 251) as u8;
    }
    let mut output = [0u8; 512];

    assert_eq!(d.write_block(3, &input), Ok(()));
    assert_eq!(d.read_block(3, &mut output), Ok(()));
    assert_eq!(output, input);
}

#[test]
fn basic_disk_blocks_are_isolated() {
    let d = Disk::new("d0", 8, 512);
    let block_three = [0x33u8; 512];
    let mut block_two = [0xFFu8; 512];
    let mut block_four = [0xFFu8; 512];

    assert_eq!(d.write_block(3, &block_three), Ok(()));
    assert_eq!(d.read_block(2, &mut block_two), Ok(()));
    assert_eq!(d.read_block(4, &mut block_four), Ok(()));
    assert!(block_two.iter().all(|&b| b == 0));
    assert!(block_four.iter().all(|&b| b == 0));
}

#[test]
fn basic_disk_rejects_wrong_buffer_size() {
    let d = Disk::new("d0", 8, 512);
    let mut short_read = [0u8; 511];
    let short_write = [0u8; 511];

    assert_eq!(d.read_block(0, &mut short_read), Err("einval"));
    assert_eq!(d.write_block(0, &short_write), Err("einval"));
}

#[test]
fn basic_disk_rejects_out_of_range_block() {
    let d = Disk::new("d0", 2, 512);
    let mut buf = [0u8; 512];

    assert_eq!(d.read_block(2, &mut buf), Err("einval"));
    assert_eq!(d.write_block(2, &buf), Err("einval"));
}

#[test]
fn basic_disk_counts_operations() {
    let d = Disk::new("d0", 2, 512);
    let mut buf = [0u8; 512];

    assert_eq!(d.total_ops(), 0);
    let _ = d.read_block(0, &mut buf);
    let _ = d.write_block(1, &buf);
    let _ = d.flush();
    assert_eq!(d.total_ops(), 3);
    d.reset_ops();
    assert_eq!(d.total_ops(), 0);
}
