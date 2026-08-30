use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

pub fn probe(socket: &Path, deadline: Duration) -> Readiness {
    let stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(_) => {
            return Readiness::NotReady(NotReady::Unreachable {
                path: socket.to_path_buf(),
            });
        }
    };
    let _ = stream.set_read_timeout(Some(deadline));
    let _ = stream.set_write_timeout(Some(deadline));
    let mut stream = stream;
    let request = "{\"id\":0,\"method\":\"status\",\"params\":{}}\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return Readiness::NotReady(NotReady::TimedOut);
    }
    let _ = stream.flush();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => Readiness::NotReady(NotReady::TimedOut),
        Ok(_) => classify(&line),
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
    use std::thread;

    const DEADLINE: Duration = Duration::from_millis(500);

    enum Answer {
        Ready(&'static str),
        NotReady(&'static str),
        Silent,
        Nonsense(&'static str),
    }

    fn serve(socket: PathBuf, answer: Answer) {
        let listener =
            UnixListener::bind(&socket).expect("the test broker binds its control socket");
        thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("the test broker accepts its one probe");
            let mut reader = BufReader::new(stream.try_clone().expect("clone for reading"));
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
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
}
