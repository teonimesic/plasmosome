/// One command of spec 025 §2's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// The words after `plasmosome` that name it, such as `["cell", "list"]`.
    pub words: &'static [&'static str],
    /// The positional arguments that may follow the words, by the names help gives them.
    pub operands: &'static [&'static str],
    /// The envelope's `command`: the wire method, or spec 020's name for a registry command.
    pub command: &'static str,
    /// Whether `command` is a wire method. A registry command has none.
    pub is_method: bool,
    /// Whether the request changes state, which decides exit 3.
    pub changes_state: bool,
    /// Whether this build serves it. An unserved command exits 2 with `not_served`.
    pub served: bool,
}

impl Command {
    /// Returns the wire method, which a `not_served` refusal names as `method`.
    pub fn method(&self) -> Option<&'static str> {
        self.is_method.then_some(self.command)
    }
}

/// Every `plasmosome` command of spec 025 §2, in the table's order. No command is served yet.
pub const COMMANDS: &[Command] = &[];
