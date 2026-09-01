use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub redacted_argv_positions: Vec<usize>,
}

impl CommandSpec {
    pub fn display(&self) -> String {
        let args = self
            .argv
            .iter()
            .enumerate()
            .map(|(index, value)| {
                if self.redacted_argv_positions.contains(&index) {
                    "<redacted>"
                } else {
                    value
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let program = self
            .program
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<redacted>");
        format!("{program} {args}")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }
}

pub trait CommandRunner {
    fn run(&mut self, command: CommandSpec) -> Result<CommandOutput, String>;
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&mut self, command: CommandSpec) -> Result<CommandOutput, String> {
        let mut child = Command::new(&command.program);
        child
            .args(&command.argv)
            .env_clear()
            .envs(&command.environment);
        if let Some(cwd) = command.cwd {
            child.current_dir(cwd);
        }
        let output = child.output().map_err(|error| error.to_string())?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Default)]
pub struct RecordingCommandRunner {
    commands: Vec<CommandSpec>,
    outputs: Vec<Result<CommandOutput, String>>,
}

impl RecordingCommandRunner {
    pub fn with_output(output: CommandOutput) -> Self {
        Self {
            commands: Vec::new(),
            outputs: vec![Ok(output)],
        }
    }
    pub fn commands(&self) -> &[CommandSpec] {
        &self.commands
    }
    pub fn scripted(outputs: Vec<Result<CommandOutput, String>>) -> Self {
        Self {
            commands: Vec::new(),
            outputs,
        }
    }
    pub fn finish(self) -> Result<(), String> {
        if self.outputs.is_empty() {
            Ok(())
        } else {
            Err("unconsumed_script_result".into())
        }
    }
}

impl CommandRunner for RecordingCommandRunner {
    fn run(&mut self, command: CommandSpec) -> Result<CommandOutput, String> {
        self.commands.push(command);
        if self.outputs.is_empty() {
            Err("unexpected_command".into())
        } else {
            self.outputs.remove(0)
        }
    }
}
