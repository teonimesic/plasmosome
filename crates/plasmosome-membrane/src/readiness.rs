use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    Ready { state: String },
    NotReady(NotReady),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotReady {
    Unreachable { path: PathBuf },
    TimedOut,
    Malformed { line: String },
    Reported { state: String },
}

impl Readiness {
    pub fn is_ready(&self) -> bool {
        matches!(self, Readiness::Ready { .. })
    }
}

const READINESS_METHOD: &str = "membrane.status";

pub fn probe(socket: &Path, deadline: Duration) -> Readiness {
    probe_with(socket, deadline, |it| UnixStream::connect(it))
}

fn probe_with(
    socket: &Path,
    deadline: Duration,
    connect: impl FnOnce(&Path) -> std::io::Result<UnixStream>,
) -> Readiness {
    let started = Instant::now();
    let stream = match connect(socket) {
        Ok(stream) => stream,
        Err(_) => {
            return Readiness::NotReady(NotReady::Unreachable {
                path: socket.to_path_buf(),
            });
        }
    };
    let Some(to_write) = deadline
        .checked_sub(started.elapsed())
        .filter(|it| !it.is_zero())
    else {
        return Readiness::NotReady(NotReady::TimedOut);
    };
    let _ = stream.set_write_timeout(Some(to_write));
    let mut stream = stream;
    let request = format!("{{\"id\":0,\"method\":\"{READINESS_METHOD}\",\"params\":{{}}}}\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return Readiness::NotReady(NotReady::TimedOut);
    }
    let _ = stream.flush();
    let Some(left) = deadline
        .checked_sub(started.elapsed())
        .filter(|it| !it.is_zero())
    else {
        return Readiness::NotReady(NotReady::TimedOut);
    };
    let _ = stream.set_read_timeout(Some(left));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => Readiness::NotReady(NotReady::TimedOut),
        Ok(_) => classify(&line),
    }
}

/// The probe a running membrane asks its brokers with: it opens the broker's
/// control socket, sends `membrane.status`, and relays what came back. Every
/// call asks again, as `brokers::Probe` requires — a kept answer cannot report a
/// broker that has since stopped serving.
pub struct ControlSocketProbe;

impl crate::brokers::Probe for ControlSocketProbe {
    fn probe(&self, socket: &Path, deadline: Duration) -> Readiness {
        probe(socket, deadline)
    }
}

fn classify(line: &str) -> Readiness {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return Readiness::NotReady(NotReady::Malformed {
            line: line.trim().to_string(),
        });
    };
    let result = value
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let state = result
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    match result.get("ready").and_then(serde_json::Value::as_bool) {
        Some(true) => Readiness::Ready { state },
        Some(false) => Readiness::NotReady(NotReady::Reported { state }),
        None => Readiness::NotReady(NotReady::Malformed {
            line: line.trim().to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::thread;

    const DEADLINE: Duration = Duration::from_millis(500);

    enum Answer {
        Ready(&'static str),
        NotReady(&'static str),
        Silent,
        Nonsense(&'static str),
    }

    fn serve(socket: PathBuf, answer: Answer) -> mpsc::Receiver<String> {
        let listener =
            UnixListener::bind(&socket).expect("the test broker binds its control socket");
        let (requests, received) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("the test broker accepts its one probe");
            let mut reader = BufReader::new(stream.try_clone().expect("clone for reading"));
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = requests.send(line);
            let reply = match answer {
                Answer::Ready(state) => {
                    format!("{{\"id\":0,\"result\":{{\"ready\":true,\"state\":\"{state}\"}}}}\n")
                }
                Answer::NotReady(state) => {
                    format!("{{\"id\":0,\"result\":{{\"ready\":false,\"state\":\"{state}\"}}}}\n")
                }
                Answer::Nonsense(text) => format!("{text}\n"),
                Answer::Silent => return,
            };
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        });
        received
    }

    fn readiness_verb_named_by_the_spec() -> String {
        let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("the membrane crate sits two levels below the workspace root")
            .join("docs")
            .join("specs")
            .join("001-control-protocol.md");
        let text = std::fs::read_to_string(&spec).unwrap_or_else(|error| {
            panic!(
                "the control protocol spec is readable at {}: {error}",
                spec.display()
            )
        });
        let section = text
            .split("\n## ")
            .find(|section| section.starts_with("4. Controller"))
            .unwrap_or_else(|| {
                panic!(
                    "{} has a section 4 naming the controller-to-membrane verbs",
                    spec.display()
                )
            });
        let bullet = section
            .lines()
            .find(|line| line.contains("the F9 readiness probe"))
            .unwrap_or_else(|| {
                panic!(
                    "{} section 4 names one verb as the F9 readiness probe",
                    spec.display()
                )
            });
        bullet
            .split('`')
            .nth(1)
            .unwrap_or_else(|| {
                panic!("the F9 readiness probe verb is quoted in backticks, got: {bullet}")
            })
            .to_string()
    }

    #[test]
    fn a_control_socket_that_answers_status_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Ready("serving"));
        let verdict = probe(&socket, DEADLINE);
        assert_eq!(
            verdict,
            Readiness::Ready {
                state: "serving".to_string()
            }
        );
        assert!(verdict.is_ready());
    }

    #[test]
    fn a_socket_that_accepts_but_never_answers_is_the_half_alive_broker() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Silent);
        let verdict = probe(&socket, DEADLINE);
        assert_eq!(verdict, Readiness::NotReady(NotReady::TimedOut));
        assert!(!verdict.is_ready());
    }

    #[test]
    fn connecting_spends_the_probe_deadline_so_a_ready_broker_still_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Ready("serving"));
        let deadline = Duration::from_millis(50);
        let verdict = probe_with(&socket, deadline, |it| {
            thread::sleep(deadline);
            UnixStream::connect(it)
        });
        assert_eq!(verdict, Readiness::NotReady(NotReady::TimedOut));
    }

    #[test]
    fn a_missing_control_socket_is_unreachable_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let verdict = probe(&dir.path().join("absent.control"), DEADLINE);
        assert!(!verdict.is_ready());
        match verdict {
            Readiness::NotReady(NotReady::Unreachable { path }) => {
                assert!(path.ends_with("absent.control"));
            }
            other => panic!("a missing socket is unreachable, got {other:?}"),
        }
    }

    #[test]
    fn an_answer_without_a_status_payload_is_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::Nonsense("{\"id\":0,\"result\":42}"));
        let verdict = probe(&socket, DEADLINE);
        assert!(!verdict.is_ready());
        assert!(matches!(
            verdict,
            Readiness::NotReady(NotReady::Malformed { .. })
        ));
    }

    #[test]
    fn a_server_reporting_not_ready_is_not_ready_even_though_it_answers() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        serve(socket.clone(), Answer::NotReady("starting"));
        let verdict = probe(&socket, DEADLINE);
        assert_eq!(
            verdict,
            Readiness::NotReady(NotReady::Reported {
                state: "starting".to_string()
            })
        );
        assert!(!verdict.is_ready());
    }

    #[test]
    fn the_production_probe_asks_the_broker_socket_and_relays_its_answer() {
        use crate::brokers::Probe;

        let dir = tempfile::tempdir().unwrap();
        let serving = dir.path().join("s.uds");
        serve(serving.clone(), Answer::Ready("serving"));
        assert_eq!(
            ControlSocketProbe.probe(&serving, DEADLINE),
            Readiness::Ready {
                state: "serving".to_string()
            }
        );

        let silent = dir.path().join("q.uds");
        serve(silent.clone(), Answer::Silent);
        assert_eq!(
            ControlSocketProbe.probe(&silent, DEADLINE),
            Readiness::NotReady(NotReady::TimedOut)
        );
    }

    #[test]
    fn the_probe_asks_for_the_verb_the_control_protocol_spec_names() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("membraned.control");
        let requests = serve(socket.clone(), Answer::Ready("serving"));
        assert!(probe(&socket, DEADLINE).is_ready());
        let line = requests
            .recv_timeout(DEADLINE)
            .expect("the test broker captured the request the probe sent");
        let request: serde_json::Value =
            serde_json::from_str(line.trim()).unwrap_or_else(|error| {
                panic!("the probe sends one JSON request per line, got {line:?}: {error}")
            });
        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("the probe's request carries a method, got {request}"));
        assert_eq!(
            method,
            readiness_verb_named_by_the_spec(),
            "the verb the readiness probe sends and the verb the control protocol spec names have diverged"
        );
    }
}
