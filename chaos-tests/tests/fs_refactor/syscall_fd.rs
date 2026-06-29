use chaos_tests::*;
use std::ffi::CString;
use std::sync::Arc;

#[test]
fn syscall_open_write_close_reopen_read_uses_swapfs() {
    let (kernel, _task) = make_kernel_and_current_task();
    let path = CString::new("/sys-hello").unwrap();
    let write_buf = b"hello syscall";
    let mut read_buf = [0u8; 32];
    let path_addr = path.as_ptr() as usize;
    let write_addr = write_buf.as_ptr() as usize;
    let read_addr = read_buf.as_mut_ptr() as usize;

    let fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_CREAT | O_RDWR, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_WRITE, fd, write_addr, 13, 0, 0, 0),
        Ok(13)
    );
    assert_eq!(kernel.dispatch_syscall(SYS_CLOSE, fd, 0, 0, 0, 0, 0), Ok(0));

    let read_fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_RDONLY, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_READ, read_fd, read_addr, 32, 0, 0, 0),
        Ok(13)
    );
    assert_eq!(&read_buf[..13], b"hello syscall");
}

#[test]
fn syscall_open_append_and_truncate_follow_fd_flags() {
    let (kernel, _task) = make_kernel_and_current_task();
    let path = CString::new("/sys-log").unwrap();
    let mut data_buf = *b"+two";
    let mut read_buf = [0u8; 16];
    let path_addr = path.as_ptr() as usize;
    let data_addr = data_buf.as_mut_ptr() as usize;
    let read_addr = read_buf.as_mut_ptr() as usize;

    data_buf[..3].copy_from_slice(b"one");
    let fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_CREAT | O_WRONLY, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_WRITE, fd, data_addr, 3, 0, 0, 0),
        Ok(3)
    );
    assert_eq!(kernel.dispatch_syscall(SYS_CLOSE, fd, 0, 0, 0, 0, 0), Ok(0));

    data_buf.copy_from_slice(b"+two");
    let append_fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_WRONLY | O_APPEND, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_WRITE, append_fd, data_addr, 4, 0, 0, 0),
        Ok(4)
    );
    assert_eq!(
        kernel.dispatch_syscall(SYS_CLOSE, append_fd, 0, 0, 0, 0, 0),
        Ok(0)
    );

    let read_fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_RDONLY, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_READ, read_fd, read_addr, 16, 0, 0, 0),
        Ok(7)
    );
    assert_eq!(&read_buf[..7], b"one+two");
    assert_eq!(
        kernel.dispatch_syscall(SYS_CLOSE, read_fd, 0, 0, 0, 0, 0),
        Ok(0)
    );

    let trunc_fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_WRONLY | O_TRUNC, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_CLOSE, trunc_fd, 0, 0, 0, 0, 0),
        Ok(0)
    );
    let empty_fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_RDONLY, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_READ, empty_fd, read_addr, 16, 0, 0, 0),
        Ok(0)
    );
}

#[test]
fn syscall_reports_faults_and_fd_errors() {
    let (kernel, _task) = make_kernel_and_current_task();
    let path = CString::new("/sys-errors").unwrap();
    let mut data_buf = *b"x";
    let path_addr = path.as_ptr() as usize;
    let data_addr = data_buf.as_mut_ptr() as usize;

    assert_eq!(
        kernel.dispatch_syscall(SYS_OPEN, path_addr, O_RDONLY, 0, 0, 0, 0),
        Err("enoent")
    );
    assert_eq!(
        kernel.dispatch_syscall(SYS_OPEN, 0, O_RDONLY, 0, 0, 0, 0),
        Err("efault")
    );
    assert_eq!(
        kernel.dispatch_syscall(SYS_WRITE, 99, 0, 1, 0, 0, 0),
        Err("efault")
    );

    let fd = kernel
        .dispatch_syscall(SYS_OPEN, path_addr, O_CREAT | O_RDONLY, 0, 0, 0, 0)
        .unwrap();
    assert_eq!(
        kernel.dispatch_syscall(SYS_WRITE, fd, data_addr, 1, 0, 0, 0),
        Err("ebadf")
    );
    assert_eq!(kernel.dispatch_syscall(SYS_CLOSE, fd, 0, 0, 0, 0, 0), Ok(0));
    assert_eq!(
        kernel.dispatch_syscall(SYS_CLOSE, fd, 0, 0, 0, 0, 0),
        Err("ebadf")
    );
}

fn make_kernel_and_current_task() -> (Kernel, Arc<Task>) {
    let kernel = Kernel::new(16);
    let task = kernel
        .tasks
        .new_user_task("/bin/test", Vec::new(), Vec::new());
    kernel.set_cur(0, Some(task.clone()));
    (kernel, task)
}
