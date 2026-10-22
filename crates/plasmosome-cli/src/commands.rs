use crate::args::Flag;

/// One command of spec 025 §2's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// The words after `plasmosome` that name it, such as `["cell", "list"]`.
    pub words: &'static [&'static str],
    /// The positional arguments that follow the words, by the names help gives them. A served
    /// command needs every one.
    pub operands: &'static [&'static str],
    /// The envelope's `command`: the wire method, or spec 020's name for a registry command.
    pub command: &'static str,
    /// Whether `command` is a wire method. A registry command has none.
    pub is_method: bool,
    /// Whether the request changes state, which decides exit 3.
    pub changes_state: bool,
    /// Whether this build serves it. An unserved command exits 2 with `not_served`.
    pub served: bool,
    /// The command's own flags. Once it is served, the command line knows them, so their values
    /// are never read as words. Each name has one kind and one key across the table.
    pub flags: &'static [Flag],
    /// Whether it takes a workload's argv after `--`.
    pub takes_argv: bool,
}

impl Command {
    /// Returns the wire method, which a `not_served` refusal names as `method`.
    pub fn method(&self) -> Option<&'static str> {
        self.is_method.then_some(self.command)
    }
}

const fn verb(
    words: &'static [&'static str],
    operands: &'static [&'static str],
    method: &'static str,
    changes_state: bool,
) -> Command {
    Command {
        words,
        operands,
        command: method,
        is_method: true,
        changes_state,
        served: false,
        flags: &[],
        takes_argv: false,
    }
}

const fn registry(
    words: &'static [&'static str],
    operands: &'static [&'static str],
    name: &'static str,
    changes_state: bool,
) -> Command {
    Command {
        words,
        operands,
        command: name,
        is_method: false,
        changes_state,
        served: false,
        flags: &[],
        takes_argv: false,
    }
}

/// Every `plasmosome` command of spec 025 §2, in the table's order. Only `status` is served.
pub const COMMANDS: &[Command] = &[
    Command {
        served: true,
        ..verb(&["status"], &[], "plasmosome.status", false)
    },
    verb(&["recovery"], &[], "plasmosome.recovery", false),
    verb(&["stop"], &[], "plasmosome.stop", true),
    verb(&["start"], &[], "plasmosome.start", true),
    verb(&["list"], &[], "plasmosome.list", false),
    verb(&["cell", "new"], &[], "cell.new", true),
    verb(&["germinate"], &["GENOME"], "cell.new", true),
    verb(&["cell", "list"], &[], "cell.list", false),
    verb(&["cell", "status"], &[], "cell.status", false),
    verb(&["cell", "kill"], &[], "cell.kill", true),
    verb(&["cell", "exec"], &[], "cell.exec", true),
    verb(&["exec", "status"], &[], "exec.status", false),
    verb(&["exec", "output"], &[], "exec.output", false),
    registry(
        &["registry", "source", "add"],
        &["ALIAS"],
        "source.add",
        false,
    ),
    registry(&["registry", "source", "list"], &[], "source.list", false),
    registry(
        &["registry", "source", "remove"],
        &["ALIAS"],
        "source.remove",
        false,
    ),
    registry(&["registry", "search"], &[], "search", false),
    registry(&["registry", "show"], &["REF"], "show", false),
    registry(&["registry", "inspect"], &["DIRECTORY"], "inspect", false),
    registry(&["registry", "fetch"], &["REF"], "fetch", false),
    registry(&["registry", "register"], &["DIRECTORY"], "register", true),
    registry(&["registry", "yank"], &["REF"], "yank", true),
    registry(&["registry", "import"], &["DIRECTORY"], "import", false),
];
