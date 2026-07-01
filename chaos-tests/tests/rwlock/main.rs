use chaos_tests::sync::rwlock::RwLock;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const WRITER_LOCKED: usize = usize::MAX;

#[test]
fn write_lock_marks_and_clears_writer_state() {
    let lock = RwLock::new();

    assert_eq!(lock.status.load(Ordering::Acquire), 0);
    lock.write_lock();
    assert_eq!(lock.status.load(Ordering::Acquire), WRITER_LOCKED);
    lock.write_unlock();
    assert_eq!(lock.status.load(Ordering::Acquire), 0);
}

#[test]
fn read_locks_increment_and_decrement_reader_count() {
    let lock = RwLock::new();

    lock.read_lock();
    assert_eq!(lock.status.load(Ordering::Acquire), 1);

    lock.read_lock();
    assert_eq!(lock.status.load(Ordering::Acquire), 2);

    lock.read_unlock();
    assert_eq!(lock.status.load(Ordering::Acquire), 1);

    lock.read_unlock();
    assert_eq!(lock.status.load(Ordering::Acquire), 0);
}

#[test]
fn writer_waits_until_readers_release() {
    let lock = Arc::new(RwLock::new());
    lock.read_lock();
    lock.read_lock();

    let (started_tx, started_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let writer_lock = Arc::clone(&lock);

    let writer = thread::spawn(move || {
        started_tx.send(()).unwrap();
        writer_lock.write_lock();
        acquired_tx.send(()).unwrap();
        writer_lock.write_unlock();
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(acquired_rx.recv_timeout(Duration::from_millis(30)).is_err());

    lock.read_unlock();
    assert!(acquired_rx.recv_timeout(Duration::from_millis(30)).is_err());

    lock.read_unlock();
    acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    writer.join().unwrap();
    assert_eq!(lock.status.load(Ordering::Acquire), 0);
}

#[test]
fn reader_waits_until_writer_releases() {
    let lock = Arc::new(RwLock::new());
    lock.write_lock();

    let (started_tx, started_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let reader_lock = Arc::clone(&lock);

    let reader = thread::spawn(move || {
        started_tx.send(()).unwrap();
        reader_lock.read_lock();
        acquired_tx.send(()).unwrap();
        reader_lock.read_unlock();
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(acquired_rx.recv_timeout(Duration::from_millis(30)).is_err());

    lock.write_unlock();
    acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    reader.join().unwrap();
    assert_eq!(lock.status.load(Ordering::Acquire), 0);
}
