use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use plasmosome_core::{InstanceRoot, LockError, WriterLock};

const ROOT_VARIABLE: &str = "PLASMOSOME_LOCK_CHILD_ROOT";
const LOCKED: &str = "plasmosome-test-writer-lock-held";
const DEADLINE: Duration = Duration::from_secs(5);

struct Holder(Child);

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Holder {
    fn reap_within(&mut self, deadline: Duration) {
        let started = Instant::now();
        while self
            .0
            .try_wait()
            .expect("the holder's status is readable")
            .is_none()
        {
            assert!(
                started.elapsed() < deadline,
                "the killed lock holder was not reaped within {deadline:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn stderr(&mut self) -> String {
        let mut text = String::new();
        if let Some(mut stderr) = self.0.stderr.take() {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    }
}

fn lock_within(instance: InstanceRoot) -> (InstanceRoot, Result<WriterLock, LockError>) {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = instance.lock();
        let _ = sender.send((instance, result));
    });
    receiver.recv_timeout(DEADLINE).unwrap_or_else(|error| {
        panic!("taking the writer lock did not return within {DEADLINE:?}: {error}")
    })
}

#[test]
#[ignore = "helper process for the lock-release test"]
fn lock_holder_child() {
    let Some(root) = std::env::var_os(ROOT_VARIABLE) else {
        return;
    };
    let instance = InstanceRoot::open(Path::new(&root)).expect("the instance root opens");
    let _lock = instance.lock().expect("the child takes the writer lock");
    println!("{LOCKED}");
    std::io::stdout().flush().expect("stdout flushes");
    let _ = std::io::stdin().read(&mut [0u8; 1]);
}

#[test]
fn process_death_releases_the_writer_lock() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir
        .path()
        .canonicalize()
        .expect("a canonical temporary path");
    let mut holder = Holder(
        Command::new(std::env::current_exe().expect("the test binary's path"))
            .args(["--exact", "lock_holder_child", "--ignored", "--nocapture"])
            .env(ROOT_VARIABLE, &root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the lock holder starts"),
    );
    let stdout = holder
        .0
        .stdout
        .take()
        .expect("the holder's stdout is piped");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains(LOCKED) {
                let _ = sender.send(());
            }
        }
    });
    if let Err(error) = receiver.recv_timeout(DEADLINE) {
        let _ = holder.0.kill();
        holder.reap_within(DEADLINE);
        panic!(
            "the holder did not report the lock within {DEADLINE:?} ({error}); its stderr:\n{}",
            holder.stderr()
        );
    }

    let instance = InstanceRoot::open(&root).expect("the instance root opens");
    let (instance, contended) = lock_within(instance);
    match contended {
        Err(LockError::Busy { path }) => assert_eq!(path, root.join("controller.lock")),
        other => panic!("expected Busy while the holder lives, got {other:?}"),
    }

    holder.0.kill().expect("the holder is killed");
    holder.reap_within(DEADLINE);
    let (_instance, released) = lock_within(instance);
    let _lock = released.expect("the holder's death released the lock");
}
