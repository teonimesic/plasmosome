use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STATUS_REQUEST: &str = r#"{"id":1,"method":"membrane.status","params":{}}"#;
const READY: &str = r#"{"id":0,"result":{"ready":true,"state":"serving"}}"#;
const STARTING: &str = r#"{"id":0,"result":{"ready":false,"state":"starting"}}"#;
const PATIENCE: Duration = Duration::from_secs(10);

fn broker_answering(socket: PathBuf, answer: &'static str) {
    let listener = UnixListener::bind(&socket).expect("the test broker binds its control socket");
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let Ok(reading) = stream.try_clone() else {
                return;
            };
            let mut line = String::new();
            let _ = BufReader::new(reading).read_line(&mut line);
            let _ = writeln!(stream, "{answer}");
            let _ = stream.flush();
        }
    });
}

struct Daemon {
    child: Child,
}

impl Daemon {
    fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    fn signal(&self, signal: libc::c_int) {
        assert_eq!(
            unsafe { libc::kill(self.pid(), signal) },
            0,
            "the daemon is still running and can be signalled"
        );
    }

    fn wait_for_exit(&mut self, budget: Duration) -> ExitStatus {
        let deadline = Instant::now() + budget;
        loop {
            match self
                .child
                .try_wait()
                .expect("the daemon's state is readable")
            {
                Some(status) => return status,
                None => assert!(
                    Instant::now() < deadline,
                    "membraned exits within {budget:?} of the signal"
                ),
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        unsafe { libc::kill(self.pid(), libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn membraned(arguments: &[&Path]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_membraned"));
    command.args(arguments).stderr(Stdio::piped());
    command
}

fn start(config: &Path) -> Daemon {
    Daemon {
        child: membraned(&[config])
            .spawn()
            .expect("membraned starts as a process"),
    }
}

fn write_config(path: &Path, body: &Value) {
    std::fs::write(path, serde_json::to_string(body).unwrap()).expect("the config is written");
}

fn addressable(socket: &Path) -> BufReader<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = UnixStream::connect(socket) {
            stream
                .set_read_timeout(Some(PATIENCE))
                .expect("the test client bounds its own reads");
            return BufReader::new(stream);
        }
        assert!(
            Instant::now() < deadline,
            "membraned is addressable on {} within five seconds",
            socket.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn ask(client: &mut BufReader<UnixStream>) -> Value {
    let mut stream = client.get_ref().try_clone().expect("clone for writing");
    writeln!(stream, "{STATUS_REQUEST}").expect("the request reaches membraned");
    stream.flush().expect("the request is flushed");
    let mut reply = String::new();
    let read = client.read_line(&mut reply).expect("membraned answers");
    assert_ne!(read, 0, "membraned answered rather than closing the socket");
    serde_json::from_str(&reply)
        .unwrap_or_else(|error| panic!("membraned answers JSON, got {reply:?}: {error}"))
}

fn recorded_pid(pidfile: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(pidfile)
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "the broker recorded its pid in {}",
            pidfile.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn is_gone(pid: i32) -> bool {
    let probed = unsafe { libc::kill(pid, 0) };
    probed == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn one_broker(dir: &Path, socket: &Path, pidfile: &Path) -> Value {
    json!({
        "control_socket": dir.join("c.uds"),
        "brokers": [{
            "name": "egressd",
            "control_socket": socket,
            "command": ["sh", "-c", format!("echo $$ > {}; exec sleep 300", pidfile.display())],
        }],
    })
}

#[test]
fn membraned_serves_ready_and_dies_cleanly_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    broker_answering(broker_socket.clone(), READY);
    write_config(&config, &one_broker(dir.path(), &broker_socket, &pidfile));

    let mut daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).get("result"),
        Some(&json!({"ready": true, "state": "serving"})),
        "membraned relays its broker set's readiness onto the wire"
    );
    let broker = recorded_pid(&pidfile);
    drop(client);

    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_for_exit(PATIENCE);
    assert_eq!(
        status.code(),
        Some(0),
        "a signalled shutdown is a clean one"
    );
    assert!(
        !control.exists(),
        "the control socket path is removed on shutdown"
    );
    assert!(
        is_gone(broker),
        "the broker died with the membrane that owned it"
    );
}

#[test]
fn membraned_reports_the_broker_that_is_not_serving() {
    let dir = tempfile::tempdir().unwrap();
    let control = dir.path().join("c.uds");
    let broker_socket = dir.path().join("b0.uds");
    let pidfile = dir.path().join("b0.pid");
    let config = dir.path().join("config.json");
    broker_answering(broker_socket.clone(), STARTING);
    write_config(&config, &one_broker(dir.path(), &broker_socket, &pidfile));

    let _daemon = start(&config);
    let mut client = addressable(&control);
    assert_eq!(
        ask(&mut client).get("result"),
        Some(
            &json!({"ready": false, "state": "not_serving", "broker": "egressd",
                     "reason": "reported", "broker_state": "starting"})
        ),
        "a broker that answers but is not serving holds the set back by name"
    );
}

#[test]
fn membraned_exits_nonzero_naming_the_failure() {
    let dir = tempfile::tempdir().unwrap();

    let no_arguments = membraned(&[]).output().expect("membraned runs");
    assert_eq!(
        no_arguments.status.code(),
        Some(2),
        "membraned needs a config"
    );
    assert!(
        String::from_utf8_lossy(&no_arguments.stderr).contains("usage"),
        "membraned says how it is called"
    );

    let absent = dir.path().join("absent.json");
    let unreadable = membraned(&[&absent]).output().expect("membraned runs");
    assert_eq!(
        unreadable.status.code(),
        Some(2),
        "an unreadable config is refused"
    );
    assert!(
        String::from_utf8_lossy(&unreadable.stderr).contains("absent.json"),
        "the refusal names the config it could not read"
    );

    let malformed = dir.path().join("malformed.json");
    std::fs::write(&malformed, b"{not json").unwrap();
    let invalid = membraned(&[&malformed]).output().expect("membraned runs");
    assert_eq!(
        invalid.status.code(),
        Some(2),
        "a config that is not JSON is refused"
    );

    let control = dir.path().join("taken.uds");
    std::fs::write(&control, b"someone else's socket").unwrap();
    let config = dir.path().join("occupied.json");
    write_config(&config, &json!({"control_socket": control}));
    let occupied = membraned(&[&config]).output().expect("membraned runs");
    assert_eq!(
        occupied.status.code(),
        Some(1),
        "a control socket path already in use refuses the start"
    );
    assert!(
        String::from_utf8_lossy(&occupied.stderr).contains("taken.uds"),
        "the refusal names the path it could not bind, got: {}",
        String::from_utf8_lossy(&occupied.stderr)
    );
}
