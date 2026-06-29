use chaos_tests::*;

#[test]
fn kernel_fd_api_opens_writes_reopens_and_reads_swapfs_file() {
    let (kernel, task_id) = make_kernel_and_task();

    let fd = kernel
        .open_file_for_task(task_id, "/hello", O_CREAT | O_RDWR, 0)
        .unwrap();
    assert_eq!(kernel.write_fd(task_id, fd, b"hello"), Ok(5));
    assert_eq!(kernel.close_fd(task_id, fd), Ok(()));

    let read_fd = kernel
        .open_file_for_task(task_id, "/hello", O_RDONLY, 0)
        .unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(kernel.read_fd(task_id, read_fd, &mut buf), Ok(5));
    assert_eq!(&buf[..5], b"hello");
}

#[test]
fn kernel_fd_api_append_uses_existing_file_size() {
    let (kernel, task_id) = make_kernel_and_task();

    let fd = kernel
        .open_file_for_task(task_id, "/log", O_CREAT | O_WRONLY, 0)
        .unwrap();
    assert_eq!(kernel.write_fd(task_id, fd, b"one"), Ok(3));
    assert_eq!(kernel.close_fd(task_id, fd), Ok(()));

    let append_fd = kernel
        .open_file_for_task(task_id, "/log", O_WRONLY | O_APPEND, 0)
        .unwrap();
    assert_eq!(kernel.write_fd(task_id, append_fd, b"+two"), Ok(4));
    assert_eq!(kernel.close_fd(task_id, append_fd), Ok(()));

    let read_fd = kernel
        .open_file_for_task(task_id, "/log", O_RDONLY, 0)
        .unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(kernel.read_fd(task_id, read_fd, &mut buf), Ok(7));
    assert_eq!(&buf[..7], b"one+two");
}

#[test]
fn kernel_fd_api_truncates_existing_file_when_opened_for_write() {
    let (kernel, task_id) = make_kernel_and_task();

    let fd = kernel
        .open_file_for_task(task_id, "/tmp", O_CREAT | O_RDWR, 0)
        .unwrap();
    assert_eq!(kernel.write_fd(task_id, fd, b"old-data"), Ok(8));
    assert_eq!(kernel.close_fd(task_id, fd), Ok(()));

    let trunc_fd = kernel
        .open_file_for_task(task_id, "/tmp", O_WRONLY | O_TRUNC, 0)
        .unwrap();
    assert_eq!(kernel.close_fd(task_id, trunc_fd), Ok(()));

    let read_fd = kernel
        .open_file_for_task(task_id, "/tmp", O_RDONLY, 0)
        .unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(kernel.read_fd(task_id, read_fd, &mut buf), Ok(0));
}

#[test]
fn kernel_fd_api_reports_open_and_permission_errors() {
    let (kernel, task_id) = make_kernel_and_task();

    assert_eq!(
        kernel.open_file_for_task(task_id, "/missing", O_RDONLY, 0),
        Err("enoent")
    );
    assert_eq!(
        kernel.open_file_for_task(usize::MAX, "/missing", O_RDONLY, 0),
        Err("esrch")
    );

    let fd = kernel
        .open_file_for_task(task_id, "/exists", O_CREAT | O_RDONLY, 0)
        .unwrap();
    assert_eq!(kernel.write_fd(task_id, fd, b"x"), Err("ebadf"));
    assert_eq!(
        kernel.open_file_for_task(task_id, "/exists", O_CREAT | O_EXCL | O_RDONLY, 0),
        Err("eexist")
    );

    let write_fd = kernel
        .open_file_for_task(task_id, "/exists", O_WRONLY, 0)
        .unwrap();
    let mut buf = [0u8; 1];
    assert_eq!(kernel.read_fd(task_id, write_fd, &mut buf), Err("ebadf"));
}

#[test]
fn kernel_fd_api_close_removes_fd_from_task_table() {
    let (kernel, task_id) = make_kernel_and_task();

    let fd = kernel
        .open_file_for_task(task_id, "/close-me", O_CREAT | O_RDWR, 0)
        .unwrap();
    assert_eq!(kernel.close_fd(task_id, fd), Ok(()));
    assert_eq!(kernel.close_fd(task_id, fd), Err("ebadf"));
    assert_eq!(kernel.write_fd(task_id, fd, b"x"), Err("ebadf"));
}

fn make_kernel_and_task() -> (Kernel, usize) {
    let kernel = Kernel::new(16);
    let task = kernel.tasks.spawn("fs-task");
    let task_id = task.id();
    (kernel, task_id)
}
