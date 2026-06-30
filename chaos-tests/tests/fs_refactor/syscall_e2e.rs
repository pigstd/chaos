use chaos_tests::*;
use std::ffi::CString;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

#[test]
fn syscall_e2e_complex_file_workload() {
    let kernel = make_shared_kernel_with_current_task();
    let shared_path = CString::new("/e2e-shared").unwrap();

    let create_fd = sys_open(&kernel, &shared_path, O_CREAT | O_WRONLY | O_TRUNC);
    sys_close(&kernel, create_fd);

    let fast_fd = sys_open(&kernel, &shared_path, O_WRONLY | O_APPEND);
    let slow_fd = sys_open(&kernel, &shared_path, O_WRONLY | O_APPEND);
    let (fast_done_tx, fast_done_rx) = mpsc::channel();

    let fast_kernel = kernel.clone();
    let fast_writer = thread::spawn(move || {
        assert_eq!(sys_write(&fast_kernel, fast_fd, b"fast\n"), 5);
        sys_close(&fast_kernel, fast_fd);
        fast_done_tx.send(()).unwrap();
    });

    let slow_kernel = kernel.clone();
    let slow_writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        fast_done_rx.recv().unwrap();
        assert_eq!(sys_write(&slow_kernel, slow_fd, b"slow\n"), 5);
        sys_close(&slow_kernel, slow_fd);
    });

    fast_writer.join().unwrap();
    slow_writer.join().unwrap();

    assert_eq!(read_file(&kernel, &shared_path, 32), b"fast\nslow\n");

    let new_path = CString::new("/e2e-new").unwrap();
    let new_fd = sys_open(&kernel, &new_path, O_CREAT | O_RDWR);
    assert_eq!(sys_write(&kernel, new_fd, b"abcdef"), 6);
    sys_close(&kernel, new_fd);
    assert_eq!(read_file(&kernel, &new_path, 16), b"abcdef");

    let overwrite_fd = sys_open(&kernel, &new_path, O_WRONLY);
    assert_eq!(sys_write(&kernel, overwrite_fd, b"XYZ"), 3);
    sys_close(&kernel, overwrite_fd);
    assert_eq!(read_file(&kernel, &new_path, 16), b"XYZdef");

    let multi_paths: Vec<CString> = (0..5)
        .map(|i| CString::new(format!("/e2e-loop-{}", i)).unwrap())
        .collect();
    let mut expected = Vec::new();
    for (i, path) in multi_paths.iter().enumerate() {
        let initial = format!("file{}:start;", i).into_bytes();
        let fd = sys_open(&kernel, path, O_CREAT | O_WRONLY | O_TRUNC);
        assert_eq!(sys_write(&kernel, fd, &initial), initial.len());
        sys_close(&kernel, fd);
        expected.push(initial);
    }

    for round in 0..4 {
        for (i, path) in multi_paths.iter().enumerate() {
            let chunk = format!("r{}f{};", round, i).into_bytes();
            let fd = sys_open(&kernel, path, O_WRONLY | O_APPEND);
            assert_eq!(sys_write(&kernel, fd, &chunk), chunk.len());
            sys_close(&kernel, fd);
            expected[i].extend_from_slice(&chunk);

            let read_back = read_file(&kernel, path, expected[i].len() + 8);
            assert_eq!(read_back, expected[i]);
        }
    }

    for (i, path) in multi_paths.iter().enumerate() {
        assert_eq!(read_file(&kernel, path, expected[i].len() + 8), expected[i]);
    }

    let path = CString::new("/e2e-large").unwrap();
    let data_len = SWAPFS_BLOCK_SIZE * 3 + 137;
    let data: Vec<u8> = (0..data_len)
        .map(|i| ((i * 31 + 7) % 251) as u8)
        .collect();

    let fd = sys_open(&kernel, &path, O_CREAT | O_WRONLY | O_TRUNC);
    assert_eq!(sys_write(&kernel, fd, &data), data_len);
    sys_close(&kernel, fd);

    let read_back = read_file(&kernel, &path, data_len + 64);
    assert_eq!(read_back.len(), data_len);
    assert_eq!(read_back, data);

    let rewrite = b"small-after-truncate";
    let trunc_fd = sys_open(&kernel, &path, O_WRONLY | O_TRUNC);
    assert_eq!(sys_write(&kernel, trunc_fd, rewrite), rewrite.len());
    sys_close(&kernel, trunc_fd);

    let read_after_trunc = read_file(&kernel, &path, data_len + 64);
    assert_eq!(read_after_trunc, rewrite);
}

fn make_shared_kernel_with_current_task() -> Arc<Kernel> {
    let kernel = Arc::new(Kernel::new(32));
    let task = kernel.tasks.new_user_task("/bin/e2e", Vec::new(), Vec::new());
    kernel.set_cur(0, Some(task));
    kernel
}

fn sys_open(kernel: &Kernel, path: &CString, flags: usize) -> usize {
    kernel
        .dispatch_syscall(SYS_OPEN, path.as_ptr() as usize, flags, 0, 0, 0, 0)
        .unwrap()
}

fn sys_write(kernel: &Kernel, fd: usize, data: &[u8]) -> usize {
    kernel
        .dispatch_syscall(SYS_WRITE, fd, data.as_ptr() as usize, data.len(), 0, 0, 0)
        .unwrap()
}

fn sys_close(kernel: &Kernel, fd: usize) {
    assert_eq!(kernel.dispatch_syscall(SYS_CLOSE, fd, 0, 0, 0, 0, 0), Ok(0));
}

fn read_file(kernel: &Kernel, path: &CString, max_len: usize) -> Vec<u8> {
    let fd = sys_open(kernel, path, O_RDONLY);
    let mut out = vec![0u8; max_len];
    let n = kernel
        .dispatch_syscall(SYS_READ, fd, out.as_mut_ptr() as usize, out.len(), 0, 0, 0)
        .unwrap();
    sys_close(kernel, fd);
    out.truncate(n);
    out
}
