use chaos_tests::*;
use std::sync::Arc;

#[test]
fn fhandle_read_write_use_swapfs_and_advance_offset() {
    let (fs, index) = make_file("alpha", 1);
    let fh = FHandle::new(
        "/alpha",
        fs.clone(),
        index,
        FdOpt {
            rd: true,
            wr: true,
            ap: false,
            nb: false,
        },
        false,
    );

    assert_eq!(fh.write(b"hello"), Ok(5));
    assert_eq!(fh.metadata_sz(), 5);
    assert_eq!(fs.metadata_len(index), Ok(5));

    assert_eq!(fh.seek(FSeek::Start(0)), Ok(0));
    let mut buf = [0u8; 8];
    assert_eq!(fh.read(&mut buf), Ok(5));
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(fh.read(&mut buf), Ok(0));
}

#[test]
fn fhandle_append_uses_swapfs_metadata_size() {
    let (fs, index) = make_file("append", 1);
    assert_eq!(fs.write_at(index, 0, b"seed"), Ok(4));

    let fh = FHandle::new(
        "/append",
        fs.clone(),
        index,
        FdOpt {
            rd: true,
            wr: true,
            ap: true,
            nb: false,
        },
        false,
    );

    assert_eq!(fh.write(b"+tail"), Ok(5));

    let mut out = [0u8; 16];
    assert_eq!(fs.read_at(index, 0, &mut out), Ok(9));
    assert_eq!(&out[..9], b"seed+tail");
}

#[test]
fn fhandle_dup_shares_offset_but_can_set_cloexec() {
    let (fs, index) = make_file("dup", 1);
    assert_eq!(fs.write_at(index, 0, b"abcdef"), Ok(6));

    let fh = FHandle::new("/dup", fs, index, FdOpt::default(), false);
    let dup = fh.dup(true);
    let mut buf = [0u8; 2];

    assert_eq!(fh.read(&mut buf), Ok(2));
    assert_eq!(&buf, b"ab");
    assert_eq!(dup.read(&mut buf), Ok(2));
    assert_eq!(&buf, b"cd");
    assert!(dup.cloexec);
}

#[test]
fn fhandle_seek_set_len_fallocate_and_splice_use_swapfs() {
    let (src_fs, src_index) = make_file("src", 1);
    let (dst_fs, dst_index) = make_file("dst", 1);

    let src = FHandle::new(
        "/src",
        src_fs.clone(),
        src_index,
        FdOpt {
            rd: true,
            wr: true,
            ap: false,
            nb: false,
        },
        false,
    );
    let dst = FHandle::new(
        "/dst",
        dst_fs.clone(),
        dst_index,
        FdOpt {
            rd: true,
            wr: true,
            ap: false,
            nb: false,
        },
        false,
    );

    assert_eq!(src.write(b"abcdef"), Ok(6));
    assert_eq!(src.seek(FSeek::End(-3)), Ok(3));
    assert_eq!(src.splice_to(&dst, 2), Ok(2));

    let mut dst_buf = [0u8; 8];
    assert_eq!(dst_fs.read_at(dst_index, 0, &mut dst_buf), Ok(2));
    assert_eq!(&dst_buf[..2], b"de");

    assert_eq!(dst.fallocate(5, 3), Ok(()));
    assert_eq!(dst.metadata_sz(), 8);
    assert_eq!(dst.set_len(1), Ok(()));
    assert_eq!(dst.metadata_sz(), 1);
    assert_eq!(dst.seek(FSeek::Cur(-1)), Ok(1));
    assert_eq!(dst.seek(FSeek::Cur(-2)), Err("einval"));
}

#[test]
fn fhandle_enforces_open_permissions() {
    let (fs, index) = make_file("perms", 1);
    let rdonly = FHandle::new("/perms", fs.clone(), index, FdOpt::default(), false);
    let wronly = FHandle::new(
        "/perms",
        fs,
        index,
        FdOpt {
            rd: false,
            wr: true,
            ap: false,
            nb: false,
        },
        false,
    );

    let mut buf = [0u8; 1];
    assert_eq!(rdonly.write(b"x"), Err("ebadf"));
    assert_eq!(rdonly.set_len(0), Err("ebadf"));
    assert_eq!(wronly.read(&mut buf), Err("ebadf"));
}

fn make_file(name: &str, initial_blocks: u64) -> (Arc<SwapFs>, usize) {
    let disk = Arc::new(Disk::new("swap0", 64, SWAPFS_BLOCK_SIZE));
    let fs = SwapFs::format(disk, 64, 8).unwrap();
    let index = fs.create(name, initial_blocks).unwrap();
    (fs, index)
}
