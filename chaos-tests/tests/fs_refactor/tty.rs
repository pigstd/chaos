use chaos_tests::*;

#[test]
fn tty_stdin_is_read_only_placeholder() {
    let tty = TtyHandle::stdin();
    let mut buf = [0xAAu8; 8];

    assert_eq!(tty.read(&mut buf), Ok(0));
    assert_eq!(tty.write(b"x"), Err("ebadf"));
    assert_eq!(tty.poll_status(), (false, false, false));
}

#[test]
fn tty_stdout_and_stderr_are_write_only_placeholders() {
    let stdout = TtyHandle::stdout();
    let stderr = TtyHandle::stderr();
    let mut buf = [0u8; 4];

    assert_eq!(stdout.write(b""), Ok(0));
    assert_eq!(stderr.write(b""), Ok(0));
    assert_eq!(stdout.read(&mut buf), Err("ebadf"));
    assert_eq!(stderr.read(&mut buf), Err("ebadf"));
    assert_eq!(stdout.poll_status(), (false, true, false));
    assert_eq!(stderr.poll_status(), (false, true, false));
}

#[test]
fn tty_dup_preserves_kind_and_sets_cloexec() {
    let stdout = FLike::Tty(TtyHandle::stdout());
    let dup = stdout.dup(true);

    match dup {
        FLike::Tty(tty) => {
            assert_eq!(tty.kind, TtyKind::Stdout);
            assert!(tty.cloexec);
        }
        _ => panic!("expected duplicated tty handle"),
    }
}

#[test]
fn new_user_task_uses_tty_for_standard_fds() {
    let kernel = Kernel::new(8);
    let task = kernel
        .tasks
        .new_user_task("/bin/init", Vec::new(), Vec::new());
    let files = task.files.lock().unwrap();

    match files.get(&0).unwrap() {
        FLike::Tty(tty) => assert_eq!(tty.kind, TtyKind::Stdin),
        _ => panic!("fd 0 should be stdin tty"),
    }
    match files.get(&1).unwrap() {
        FLike::Tty(tty) => assert_eq!(tty.kind, TtyKind::Stdout),
        _ => panic!("fd 1 should be stdout tty"),
    }
    match files.get(&2).unwrap() {
        FLike::Tty(tty) => assert_eq!(tty.kind, TtyKind::Stderr),
        _ => panic!("fd 2 should be stderr tty"),
    }
}
