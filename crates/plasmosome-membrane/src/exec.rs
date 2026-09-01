use crate::vmm::Launch;
use std::ffi::CString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

/// Why an argv could not be turned into a command the child can exec.
#[derive(Debug)]
pub enum ExecError {
    EmptyArgv,
    NulInArgument { argument: String },
    ProgramNotFound { program: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::EmptyArgv => write!(f, "a command needs at least a program to run"),
            ExecError::NulInArgument { argument } => {
                write!(f, "argument `{argument}` holds a NUL byte")
            }
            ExecError::ProgramNotFound { program } => {
                write!(f, "program `{program}` was not found")
            }
        }
    }
}

impl std::error::Error for ExecError {}

/// A command resolved and laid out in the parent, ready to be `execve`d in a
/// forked child.
///
/// Everything that allocates — resolving the program against `PATH`, snapshotting
/// the environment, building the C strings and the two pointer arrays — happens in
/// `new`, in the parent. The child body is then only `execve` and `_exit`, which is
/// what the crate's after-fork rule requires: the parent is multi-threaded, so an
/// allocation in the child can deadlock on a lock another thread held at fork time.
///
/// A caller passes the whole argv, program first. `new` refuses an argv the exec
/// could never carry, so a command that cannot run is refused before any fork.
pub struct ExecCommand {
    program: CString,
    arguments: Vec<CString>,
    environment: Vec<CString>,
    argv: Vec<*const libc::c_char>,
    envp: Vec<*const libc::c_char>,
}

impl ExecCommand {
    /// Resolves `argv` into a command, or says why it cannot be one. `argv[0]`
    /// holding a `/` is taken as a path and must exist; otherwise it is looked up
    /// in `PATH`. A program that exists but cannot be executed is not refused here
    /// — that failure is only visible to `execve`, and shows up as the child
    /// exiting 127.
    pub fn new(argv: Vec<String>) -> Result<ExecCommand, ExecError> {
        let Some(name) = argv.first().cloned() else {
            return Err(ExecError::EmptyArgv);
        };
        let mut arguments = Vec::with_capacity(argv.len());
        for argument in argv {
            match CString::new(argument.as_bytes()) {
                Ok(carried) => arguments.push(carried),
                Err(_) => return Err(ExecError::NulInArgument { argument }),
            }
        }
        let program = resolve(&name).ok_or(ExecError::ProgramNotFound { program: name })?;
        let environment: Vec<CString> = std::env::vars_os()
            .filter_map(|(key, value)| {
                let mut entry = key.into_vec();
                entry.push(b'=');
                entry.extend_from_slice(value.as_bytes());
                CString::new(entry).ok()
            })
            .collect();
        let mut command = ExecCommand {
            program,
            arguments,
            environment,
            argv: Vec::new(),
            envp: Vec::new(),
        };
        command.argv = command
            .arguments
            .iter()
            .map(|argument| argument.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        command.envp = command
            .environment
            .iter()
            .map(|entry| entry.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        Ok(command)
    }
}

fn resolve(program: &str) -> Option<CString> {
    if program.contains('/') {
        if !Path::new(program).exists() {
            return None;
        }
        return CString::new(program.as_bytes()).ok();
    }
    let search = std::env::var_os("PATH")?;
    let found = std::env::split_paths(&search)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())?;
    CString::new(found.into_os_string().into_vec()).ok()
}

impl std::fmt::Debug for ExecCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecCommand")
            .field("program", &self.program)
            .field("arguments", &self.arguments)
            .field("environment", &self.environment.len())
            .finish_non_exhaustive()
    }
}

impl Launch for ExecCommand {
    fn launch(self) -> ! {
        unsafe {
            libc::execve(
                self.program.as_ptr(),
                self.argv.as_ptr(),
                self.envp.as_ptr(),
            );
            libc::_exit(127)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmm::{VmmChild, VmmState};
    use std::io::Write;
    use std::time::Duration;

    const DEADLINE: Duration = Duration::from_secs(5);

    fn run(argv: &[&str]) -> VmmState {
        let command = ExecCommand::new(argv.iter().map(|word| word.to_string()).collect())
            .unwrap_or_else(|error| panic!("{argv:?} resolves into a command: {error}"));
        let mut child = VmmChild::spawn(command).expect("the fork succeeds");
        child.wait_terminal(DEADLINE)
    }

    #[test]
    fn an_exec_command_runs_the_program_and_its_exit_code_comes_back() {
        assert_eq!(run(&["sh", "-c", "exit 7"]), VmmState::Exited { code: 7 });
    }

    #[test]
    fn argv_carries_every_argument_in_order() {
        assert_eq!(
            run(&["sh", "-c", "exit $1", "x", "9"]),
            VmmState::Exited { code: 9 }
        );
    }

    #[test]
    fn the_childs_environment_is_the_parents_snapshot() {
        assert_eq!(
            run(&["sh", "-c", "test -n \"$PATH\""]),
            VmmState::Exited { code: 0 }
        );
    }

    #[test]
    fn arguments_the_exec_cannot_carry_are_refused() {
        match ExecCommand::new(Vec::new()) {
            Err(ExecError::EmptyArgv) => {}
            other => panic!("an empty argv is refused, got {:?}", other.map(|_| ())),
        }
        match ExecCommand::new(vec!["sh".to_string(), "a\0b".to_string()]) {
            Err(ExecError::NulInArgument { argument }) => assert_eq!(argument, "a\0b"),
            other => panic!(
                "an argument holding a NUL is refused, got {:?}",
                other.map(|_| ())
            ),
        }
        match ExecCommand::new(vec!["plasmosome-no-such-program".to_string()]) {
            Err(ExecError::ProgramNotFound { program }) => {
                assert_eq!(program, "plasmosome-no-such-program");
            }
            other => panic!(
                "a program that is nowhere on PATH is refused, got {:?}",
                other.map(|_| ())
            ),
        }
    }

    #[test]
    fn a_program_that_cannot_be_executed_exits_the_child_with_127() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("not-executable");
        let mut file = std::fs::File::create(&program).expect("the test writes a plain file");
        file.write_all(b"this is not a program\n").unwrap();
        drop(file);
        let command = ExecCommand::new(vec![program.display().to_string()])
            .expect("a file that exists resolves, whether or not it can be executed");
        let mut child = VmmChild::spawn(command).expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(DEADLINE),
            VmmState::Exited { code: 127 }
        );
    }
}
