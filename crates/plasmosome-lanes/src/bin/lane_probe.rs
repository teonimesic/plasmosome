use std::process::ExitCode;

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("lane-probe: refused: runs only in a Linux guest");
    ExitCode::from(2)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    linux::main(&args)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{self, File};
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::os::unix::fs::{FileExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitCode, Stdio};

    use plasmosome_lanes::bundle::sha256_hex;
    use plasmosome_lanes::guest_sys::{self, Mapping};
    use plasmosome_lanes::probe::{self, failure};
    use serde_json::{Value, json};

    const HOLD_DIR: &str = "/run/lane-hold";
    const USAGE: u8 = 64;

    fn print(line: &Value) {
        println!("{line}");
    }

    fn read_text(probe: &str, path: &str, parse: fn(&str) -> Value) -> Value {
        fs::read_to_string(path).map_or_else(|error| failure(probe, &error), |text| parse(&text))
    }

    fn uname() -> Value {
        match guest_sys::uname() {
            Ok(uname) => json!({
                "probe": "uname", "ok": true, "sysname": uname.sysname,
                "nodename": uname.nodename, "release": uname.release,
                "version": uname.version, "machine": uname.machine,
            }),
            Err(error) => failure("uname", &error),
        }
    }

    fn socket(name: &str) -> PathBuf {
        Path::new(HOLD_DIR).join(format!("{name}.sock"))
    }

    fn read_all_at(file: &File) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let read = file.read_at(&mut chunk, bytes.len() as u64)?;
            if read == 0 {
                return Ok(bytes);
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
    }

    enum Held {
        Fd(File),
        Map(Mapping),
        Exec,
    }

    impl Held {
        fn kind(&self) -> &'static str {
            match self {
                Held::Fd(_) => "fd",
                Held::Map(_) => "map",
                Held::Exec => "exec",
            }
        }

        fn read(&self) -> io::Result<Vec<u8>> {
            match self {
                Held::Fd(file) => read_all_at(file),
                Held::Map(mapping) => Ok(mapping.copy_out()),
                Held::Exec => fs::read("/proc/self/exe"),
            }
        }
    }

    fn hold(kind: &str, path: &str) -> io::Result<Held> {
        match kind {
            "fd" => File::open(path).map(Held::Fd),
            "map" => Mapping::map(&File::open(path)?).map(Held::Map),
            _ => Ok(Held::Exec),
        }
    }

    fn serve(kind: &str, name: &str, path: &str) -> ExitCode {
        let probe = format!("hold-{kind}");
        let held = match hold(kind, path) {
            Ok(held) => held,
            Err(error) => {
                print(&failure(&probe, &error));
                return ExitCode::SUCCESS;
            }
        };
        let place = socket(name);
        let listener = match UnixListener::bind(&place).and_then(|listener| {
            fs::set_permissions(&place, fs::Permissions::from_mode(0o666)).map(|()| listener)
        }) {
            Ok(listener) => listener,
            Err(error) => {
                print(&failure(&probe, &error));
                return ExitCode::SUCCESS;
            }
        };
        print(&json!({
            "probe": probe, "ok": true, "name": name, "kind": held.kind(), "pid": std::process::id(),
        }));
        let _ = io::stdout().flush();
        let Ok((mut stream, _)) = listener.accept() else {
            return ExitCode::FAILURE;
        };
        let reply = match held.read() {
            Ok(bytes) => json!({
                "probe": "release", "ok": true, "name": name, "kind": held.kind(),
                "bytes": bytes.len(), "sha256": sha256_hex(&bytes),
            }),
            Err(error) => {
                let mut line = failure("release", &error);
                line["name"] = json!(name);
                line
            }
        };
        let _ = stream.write_all(format!("{reply}\n").as_bytes());
        let _ = fs::remove_file(&place);
        ExitCode::SUCCESS
    }

    fn start_holder(probe: &str, program: &str, args: &[&str]) -> Value {
        let spawned = Command::new(program)
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return failure(probe, &error),
        };
        let mut line = String::new();
        let read = child
            .stdout
            .take()
            .map(|stdout| BufReader::new(stdout.take(64 * 1024)).read_line(&mut line));
        match (read, serde_json::from_str::<Value>(line.trim_end())) {
            (Some(Ok(_)), Ok(value)) if value.is_object() => value,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                json!({"probe": probe, "ok": false, "error": "the holder printed no ready line"})
            }
        }
    }

    fn release(name: &str) -> Value {
        let mut stream = match UnixStream::connect(socket(name)) {
            Ok(stream) => stream,
            Err(error) => return failure("release", &error),
        };
        let mut reply = String::new();
        let _ = stream.read_to_string(&mut reply);
        serde_json::from_str(reply.trim_end()).unwrap_or_else(|_| {
            json!({"probe": "release", "ok": false, "name": name,
                   "error": "the holder closed without a reply"})
        })
    }

    fn observe(args: &[&str]) -> Option<Value> {
        let exe = "/proc/self/exe";
        Some(match args {
            ["stat", path] => probe::stat(Path::new(path)),
            ["write", path] => probe::write(Path::new(path)),
            ["create", path] => probe::create(Path::new(path)),
            ["list", path] => probe::list(Path::new(path)),
            ["lookup", path] => probe::lookup(Path::new(path)),
            ["id"] => read_text("id", "/proc/self/status", probe::id_from_status),
            ["filesystems"] => {
                read_text("filesystems", "/proc/filesystems", probe::filesystems_from)
            }
            ["mountinfo"] => read_text("mountinfo", "/proc/self/mountinfo", probe::mountinfo_from),
            ["uname"] => uname(),
            ["hold-fd", name, path] => start_holder("hold-fd", exe, &["serve", "fd", name, path]),
            ["hold-map", name, path] => {
                start_holder("hold-map", exe, &["serve", "map", name, path])
            }
            ["hold-exec", name, path] => {
                start_holder("hold-exec", path, &["serve", "exec", name, path])
            }
            ["release", name] => release(name),
            _ => return None,
        })
    }

    /// Run one subcommand and print its one JSON line.
    pub fn main(args: &[String]) -> ExitCode {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        if let ["serve", kind @ ("fd" | "map" | "exec"), name, path] = args.as_slice() {
            return serve(kind, name, path);
        }
        match observe(&args) {
            Some(line) => {
                print(&line);
                ExitCode::SUCCESS
            }
            None => {
                print(
                    &json!({"probe": args.first().copied().unwrap_or(""), "ok": false,
                              "error": "unknown subcommand or wrong arguments"}),
                );
                ExitCode::from(USAGE)
            }
        }
    }
}
