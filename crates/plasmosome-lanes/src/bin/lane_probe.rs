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
    use std::os::fd::AsFd;
    use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitCode, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use plasmosome_lanes::bundle::sha256_hex;
    use plasmosome_lanes::config::WORKLOAD_UID;
    use plasmosome_lanes::guest_sys::{
        self, CLONE_FILES, CLONE_FS, CLONE_NEWNS, MNT_DETACH, MS_BIND, MS_PRIVATE, MS_SHARED,
        MS_SLAVE, Mapping,
    };
    use plasmosome_lanes::probe::{self, failure};
    use serde_json::{Value, json};

    const HOLD_DIR: &str = "/run/lane-hold";
    const USAGE: u8 = 64;
    const SELF: &str = "/proc/self/exe";
    const POLL: Duration = Duration::from_millis(5);

    fn print(line: &Value) {
        let mut line = line.clone();
        if let Some(fields) = line.as_object_mut() {
            fields
                .entry("id_u")
                .or_insert_with(|| json!(guest_sys::effective_uid()));
        }
        println!("{line}");
    }

    fn millis(start: Instant) -> u64 {
        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn read_text(probe: &str, path: &str, parse: fn(&str) -> Value) -> Value {
        fs::read_to_string(path).map_or_else(|error| failure(probe, &error), |text| parse(&text))
    }

    fn done(probe: &str, result: io::Result<()>) -> Value {
        match result {
            Ok(()) => json!({"probe": probe, "ok": true}),
            Err(error) => failure(probe, &error),
        }
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

    fn socket(name: &str) -> io::Result<PathBuf> {
        if probe::holder_name_ok(name) {
            return Ok(Path::new(HOLD_DIR).join(format!("{name}.sock")));
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "holder name `{name}` must start with a letter or digit and hold only letters, digits, `.`, `_` and `-`"
            ),
        ))
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

    type Reply = mpsc::Sender<io::Result<Vec<u8>>>;

    enum Held {
        Fd(File),
        Map(Mapping),
        Exec,
        Cwd(Vec<String>),
        Watch {
            path: PathBuf,
            before: Value,
        },
        Thread {
            requests: mpsc::Sender<Reply>,
            tid: u64,
        },
        InFlight {
            _sender: UnixStream,
            receiver: UnixStream,
        },
    }

    impl Held {
        fn kind(&self) -> &'static str {
            match self {
                Held::Fd(_) => "fd",
                Held::Map(_) => "map",
                Held::Exec => "exec",
                Held::Cwd(_) => "cwd",
                Held::Watch { .. } => "watch",
                Held::Thread { .. } => "thread",
                Held::InFlight { .. } => "inflight",
            }
        }

        fn annotate(&self, line: &mut Value) {
            match self {
                Held::Cwd(former) => line["former"] = json!(former.len()),
                Held::Watch { before, .. } => line["before"] = before.clone(),
                Held::Thread { tid, .. } => line["tid"] = json!(tid),
                _ => {}
            }
        }

        fn read(&self) -> io::Result<Vec<u8>> {
            match self {
                Held::Fd(file) => read_all_at(file),
                Held::Map(mapping) => Ok(mapping.copy_out()),
                Held::Exec => fs::read(SELF),
                Held::Thread { requests, .. } => {
                    let (reply, answer) = mpsc::channel();
                    requests
                        .send(reply)
                        .map_err(|_| io::Error::other("the holding thread is gone"))?;
                    answer
                        .recv()
                        .map_err(|_| io::Error::other("the holding thread did not answer"))?
                }
                Held::InFlight { receiver, .. } => {
                    read_all_at(&File::from(guest_sys::recv_fd(receiver)?))
                }
                Held::Cwd(_) | Held::Watch { .. } => {
                    Err(io::Error::from(io::ErrorKind::Unsupported))
                }
            }
        }

        fn reply(&self, name: &str) -> Value {
            let mut line = match self {
                Held::Cwd(former) => {
                    let lookups: Vec<Value> = former
                        .iter()
                        .map(|name| json!({"name": name, "result": probe::lookup(Path::new(name))}))
                        .collect();
                    json!({"probe": "release", "ok": true,
                           "listing": probe::list(Path::new(".")), "lookups": lookups})
                }
                Held::Watch { path, .. } => {
                    json!({"probe": "release", "ok": true, "lookup": probe::lookup(path)})
                }
                _ => match self.read() {
                    Ok(bytes) => json!({"probe": "release", "ok": true,
                                        "bytes": bytes.len(), "sha256": sha256_hex(&bytes)}),
                    Err(error) => failure("release", &error),
                },
            };
            line["name"] = json!(name);
            line["kind"] = json!(self.kind());
            line["id_u"] = json!(guest_sys::effective_uid());
            line
        }
    }

    fn walk(base: &Path, prefix: &str, names: &mut Vec<String>) -> io::Result<()> {
        let mut entries = fs::read_dir(base)?
            .map(|entry| {
                entry?.file_name().into_string().map_err(|name| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("the name {name:?} is not UTF-8"),
                    )
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        for name in entries {
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let path = base.join(&name);
            names.push(relative.clone());
            if fs::symlink_metadata(&path)?.is_dir() {
                walk(&path, &relative, names)?;
            }
        }
        Ok(())
    }

    fn own_tid() -> io::Result<u64> {
        fs::read_link("/proc/thread-self")?
            .file_name()
            .and_then(|tid| tid.to_str())
            .and_then(|tid| tid.parse().ok())
            .ok_or_else(|| io::Error::other("/proc/thread-self names no task"))
    }

    fn hold_in_thread(file: &str, dir: &str) -> io::Result<Held> {
        let (requests, incoming) = mpsc::channel::<Reply>();
        let (ready, started) = mpsc::channel::<io::Result<u64>>();
        let (file, dir) = (file.to_string(), dir.to_string());
        thread::spawn(move || {
            let setup = guest_sys::unshare(CLONE_FS | CLONE_FILES)
                .and_then(|()| File::open(&file))
                .and_then(|held| std::env::set_current_dir(&dir).map(|()| held))
                .and_then(|held| own_tid().map(|tid| (held, tid)));
            match setup {
                Ok((held, tid)) => {
                    let _ = ready.send(Ok(tid));
                    for reply in incoming {
                        let _ = reply.send(read_all_at(&held));
                    }
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            }
        });
        let tid = started
            .recv()
            .map_err(|_| io::Error::other("the holding thread ended"))??;
        Ok(Held::Thread { requests, tid })
    }

    fn hold_in_flight(file: &str) -> io::Result<Held> {
        let held = File::open(file)?;
        let (sender, receiver) = UnixStream::pair()?;
        guest_sys::send_fd(&sender, held.as_fd())?;
        drop(held);
        Ok(Held::InFlight {
            _sender: sender,
            receiver,
        })
    }

    fn hold(kind: &str, paths: &[&str]) -> io::Result<Held> {
        match (kind, paths) {
            ("fd", [path]) => File::open(path).map(Held::Fd),
            ("map", [path]) => Mapping::map(&File::open(path)?).map(Held::Map),
            ("exec", [_]) => Ok(Held::Exec),
            ("cwd", [dir]) => {
                std::env::set_current_dir(dir)?;
                let mut former = Vec::new();
                walk(Path::new("."), "", &mut former)?;
                Ok(Held::Cwd(former))
            }
            ("watch", [path]) => Ok(Held::Watch {
                path: PathBuf::from(path),
                before: probe::lookup(Path::new(path)),
            }),
            ("thread", [file, dir]) => hold_in_thread(file, dir),
            ("inflight", [file]) => hold_in_flight(file),
            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }

    fn serve(kind: &str, name: &str, paths: &[&str]) -> ExitCode {
        let probe = format!("hold-{kind}");
        let place = match socket(name) {
            Ok(place) => place,
            Err(error) => {
                print(&failure(&probe, &error));
                return ExitCode::SUCCESS;
            }
        };
        let held = match hold(kind, paths) {
            Ok(held) => held,
            Err(error) => {
                print(&failure(&probe, &error));
                return ExitCode::SUCCESS;
            }
        };
        let listener = match UnixListener::bind(&place).and_then(|listener| {
            fs::set_permissions(&place, fs::Permissions::from_mode(0o666)).map(|()| listener)
        }) {
            Ok(listener) => listener,
            Err(error) => {
                print(&failure(&probe, &error));
                return ExitCode::SUCCESS;
            }
        };
        let mut ready = json!({
            "probe": probe, "ok": true, "name": name, "kind": held.kind(), "pid": std::process::id(),
        });
        held.annotate(&mut ready);
        print(&ready);
        let _ = io::stdout().flush();
        let Ok((mut stream, _)) = listener.accept() else {
            return ExitCode::FAILURE;
        };
        let reply = held.reply(name);
        let _ = stream.write_all(format!("{reply}\n").as_bytes());
        let _ = fs::remove_file(&place);
        ExitCode::SUCCESS
    }

    fn start_holder(probe: &str, program: &str, kind: &str, name: &str, paths: &[&str]) -> Value {
        if let Err(error) = socket(name) {
            return failure(probe, &error);
        }
        let spawned = Command::new(program)
            .args(["serve", kind, name])
            .args(paths)
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
        let place = match socket(name) {
            Ok(place) => place,
            Err(error) => return failure("release", &error),
        };
        let mut stream = match UnixStream::connect(place) {
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

    fn await_path(path: &str, limit_ms: u64) -> Value {
        let start = Instant::now();
        loop {
            if fs::symlink_metadata(path).is_ok() {
                return json!({"probe": "await", "ok": true, "elapsed_ms": millis(start)});
            }
            if millis(start) >= limit_ms {
                return json!({"probe": "await", "ok": false, "error": "timed out",
                              "elapsed_ms": millis(start)});
            }
            thread::sleep(POLL);
        }
    }

    fn sleep(ms: u64) -> Value {
        let start = Instant::now();
        thread::sleep(Duration::from_millis(ms));
        json!({"probe": "sleep", "ok": true, "slept_ms": millis(start)})
    }

    type Target = (String, u64, u64);

    fn matching(targets: &[Target], path: &Path) -> Vec<String> {
        let Ok(metadata) = fs::metadata(path) else {
            return Vec::new();
        };
        targets
            .iter()
            .filter(|(_, dev, ino)| metadata.dev() == *dev && metadata.ino() == *ino)
            .map(|(target, _, _)| target.clone())
            .collect()
    }

    fn references(base: &Path, targets: &[Target]) -> Vec<(String, String)> {
        let mut found = Vec::new();
        if let Ok(fds) = fs::read_dir(base.join("fd")) {
            for fd in fds.flatten() {
                let entry = format!("fd/{}", fd.file_name().to_string_lossy());
                for target in matching(targets, &fd.path()) {
                    found.push((entry.clone(), target));
                }
            }
        }
        for entry in ["cwd", "root", "exe"] {
            for target in matching(targets, &base.join(entry)) {
                found.push((entry.to_string(), target));
            }
        }
        if let Ok(text) = fs::read_to_string(base.join("maps")) {
            let mapped = probe::map_identities(&text);
            for (target, dev, ino) in targets {
                let device = format!("{:02x}:{:02x}", libc::major(*dev), libc::minor(*dev));
                if mapped
                    .iter()
                    .any(|(seen, inode)| *seen == device && inode == ino)
                {
                    found.push(("maps".to_string(), target.clone()));
                }
            }
        }
        found
    }

    fn numeric(name: &std::ffi::OsStr) -> Option<u64> {
        name.to_str().and_then(|name| name.parse().ok())
    }

    fn scan(paths: &[&str]) -> Value {
        let targets = match paths
            .iter()
            .map(|path| {
                fs::metadata(path).map(|found| (path.to_string(), found.dev(), found.ino()))
            })
            .collect::<io::Result<Vec<Target>>>()
        {
            Ok(targets) => targets,
            Err(error) => return failure("scan", &error),
        };
        let processes = match fs::read_dir("/proc") {
            Ok(processes) => processes,
            Err(error) => return failure("scan", &error),
        };
        let me = u64::from(std::process::id());
        let mut hits = Vec::new();
        for process in processes.flatten() {
            let Some(pid) = numeric(&process.file_name()).filter(|pid| *pid != me) else {
                continue;
            };
            for (entry, target) in references(&process.path(), &targets) {
                hits.push(json!({"pid": pid, "tid": null, "view": "process",
                                 "entry": entry, "target": target}));
            }
            let Ok(tasks) = fs::read_dir(process.path().join("task")) else {
                continue;
            };
            for task in tasks.flatten() {
                let Some(tid) = numeric(&task.file_name()) else {
                    continue;
                };
                for (entry, target) in references(&task.path(), &targets) {
                    hits.push(json!({"pid": pid, "tid": tid, "view": "task",
                                     "entry": entry, "target": target}));
                }
            }
        }
        let targets: Vec<Value> = targets
            .iter()
            .map(|(path, dev, ino)| json!({"path": path, "dev": dev, "ino": ino}))
            .collect();
        json!({"probe": "scan", "ok": true, "targets": targets, "hits": hits})
    }

    fn placed(dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir.join("p/bin"))?;
        fs::write(dir.join("p/bin/x"), b"x\n")
    }

    fn read_line_json(reader: &mut impl BufRead) -> io::Result<Value> {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        serde_json::from_str(line.trim_end()).map_err(io::Error::other)
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }

    fn staged(config: &str) -> io::Result<Value> {
        let base = PathBuf::from(format!("/run/r2-{config}"));
        let source = PathBuf::from(format!("/run/r2-source-{config}"));
        let hidden = PathBuf::from(format!("/run/r2-hidden-{config}"));
        let unreachable = config == "unreachable";
        let parent_private = matches!(config, "private-parent" | "late-directory");
        let late = config == "late-directory";
        fs::create_dir_all(base.join(if late { "opt" } else { "opt/p" }))?;
        guest_sys::mount(&text(&base), &base, "none", MS_BIND)?;
        let parent = if parent_private {
            MS_PRIVATE
        } else {
            MS_SHARED
        };
        guest_sys::mount("none", &base, "none", parent)?;
        if unreachable {
            fs::create_dir_all(source.join("p"))?;
            placed(&hidden)?;
        } else {
            placed(&source)?;
        }
        let mut child = Command::new(SELF)
            .args(["ns-workload", config, &text(&base), &text(&source)])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the workload's pipes are missing"));
        };
        let mut stdout = BufReader::new(stdout);
        let ready = read_line_json(&mut stdout)?;
        if ready["ok"] != true {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(json!({"probe": "stage", "ok": false, "config": config,
                             "error": format!("the workload namespace was not made: {ready}")}));
        }
        let mount = if unreachable {
            guest_sys::mount(&text(&hidden), &source, "none", MS_BIND)?;
            None
        } else {
            if late {
                fs::create_dir(base.join("opt/p"))?;
            }
            Some(guest_sys::mount(
                &text(&source.join("p")),
                &base.join("opt/p"),
                "none",
                MS_BIND,
            ))
        };
        writeln!(stdin, "go")?;
        let reply = read_line_json(&mut stdout)?;
        let _ = child.wait();
        let (mount, placed_by, initial_path) = match mount {
            Some(result) => (
                done("mount", result),
                "initial namespace",
                base.join("opt/p/bin/x"),
            ),
            None => (
                reply["bind"].clone(),
                "workload namespace",
                source.join("p/bin/x"),
            ),
        };
        let attach_point = if late {
            "created after the workload namespace was made"
        } else {
            "made before the workload namespace"
        };
        Ok(
            json!({"probe": "stage", "ok": true, "config": config, "placed_by": placed_by,
                  "parent": if parent_private { "private" } else { "shared" },
                  "workload_propagation": workload_propagation(config).1,
                  "attach_point": attach_point,
                  "mount": mount, "initial_stat": probe::stat(&initial_path),
                  "workload_stat": reply["workload_stat"]}),
        )
    }

    fn stage(config: &str) -> Value {
        staged(config).unwrap_or_else(|error| failure("stage", &error))
    }

    fn workload_propagation(config: &str) -> (Option<libc::c_ulong>, &'static str) {
        match config {
            "private" => (Some(MS_PRIVATE), "private"),
            "private-parent" | "late-directory" => (None, "left as unshare made it"),
            _ => (Some(MS_SLAVE), "slave"),
        }
    }

    fn ns_workload(config: &str, base: &str, source: &str) -> ExitCode {
        let setup =
            guest_sys::unshare(CLONE_NEWNS).and_then(|()| match workload_propagation(config).0 {
                Some(flags) => guest_sys::mount("none", Path::new(base), "none", flags),
                None => Ok(()),
            });
        print(&done("ns-workload", setup));
        let _ = io::stdout().flush();
        let mut go = String::new();
        if io::stdin().read_line(&mut go).is_err() {
            return ExitCode::FAILURE;
        }
        let bind = (config == "unreachable").then(|| {
            done(
                "mount",
                guest_sys::mount(
                    &format!("{source}/p"),
                    &Path::new(base).join("opt/p"),
                    "none",
                    MS_BIND,
                ),
            )
        });
        let stat = Command::new(SELF)
            .arg("stat")
            .arg(format!("{base}/opt/p/bin/x"))
            .env_clear()
            .uid(WORKLOAD_UID)
            .gid(WORKLOAD_UID)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let workload_stat = stat
            .ok()
            .and_then(|output| serde_json::from_slice::<Value>(output.stdout.trim_ascii_end()).ok())
            .unwrap_or_else(
                || json!({"probe": "stat", "ok": false, "error": "the workload's stat printed no line"}),
            );
        print(&json!({"probe": "ns-workload", "ok": true, "bind": bind,
                      "workload_stat": workload_stat}));
        ExitCode::SUCCESS
    }

    fn mount(source: &str, target: &str, fstype: &str) -> Value {
        let target = Path::new(target);
        done(
            "mount",
            fs::create_dir_all(target).and_then(|()| guest_sys::mount(source, target, fstype, 0)),
        )
    }

    fn observe(args: &[&str]) -> Option<Value> {
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
            ["devices"] => probe::devices(Path::new("/sys")),
            ["hold-fd", name, path] => start_holder("hold-fd", SELF, "fd", name, &[path]),
            ["hold-map", name, path] => start_holder("hold-map", SELF, "map", name, &[path]),
            ["hold-exec", name, path] => start_holder("hold-exec", path, "exec", name, &[path]),
            ["hold-cwd", name, dir] => start_holder("hold-cwd", SELF, "cwd", name, &[dir]),
            ["hold-watch", name, path] => start_holder("hold-watch", SELF, "watch", name, &[path]),
            ["hold-thread", name, file, dir] => {
                start_holder("hold-thread", SELF, "thread", name, &[file, dir])
            }
            ["hold-inflight", name, file] => {
                start_holder("hold-inflight", SELF, "inflight", name, &[file])
            }
            ["release", name] => release(name),
            ["await", path, ms] => await_path(path, ms.parse().ok()?),
            ["sleep", ms] => sleep(ms.parse().ok()?),
            ["mount", source, target, fstype] => mount(source, target, fstype),
            ["umount-detach", target] => done(
                "umount-detach",
                guest_sys::umount2(Path::new(target), MNT_DETACH),
            ),
            ["scan", paths @ ..] if !paths.is_empty() => scan(paths),
            [
                "stage",
                config @ ("visible" | "private" | "unreachable" | "private-parent"
                | "late-directory"),
            ] => stage(config),
            _ => return None,
        })
    }

    /// Run one subcommand and print its one JSON line.
    pub fn main(args: &[String]) -> ExitCode {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match args.as_slice() {
            [
                "serve",
                kind @ ("fd" | "map" | "exec" | "cwd" | "watch" | "thread" | "inflight"),
                name,
                paths @ ..,
            ] => return serve(kind, name, paths),
            ["ns-workload", config, base, source] => return ns_workload(config, base, source),
            _ => {}
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
