use std::ffi::OsStr;

use crate::json::{Json, Number};

/// One of a command's own flags: a wire parameter written as a long flag, as spec 025 section 2
/// says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flag {
    /// The flag as written, such as `--lease-ms`.
    pub name: &'static str,
    /// The wire parameter it sets, such as `lease_ms`.
    pub key: &'static str,
    pub kind: FlagKind,
}

/// What a command's flag takes and what it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagKind {
    /// `--name VALUE`: the next argument, sent as a string unchanged.
    Text,
    /// `--name N`, where N is `0` or digits with no leading zero, sent as a number with N's text.
    Integer,
    /// `--name` alone: `true` when given, and no key when not.
    Bare,
    /// `--mock` alone sends `"simulate"`, and `--mock=MODE` sends MODE unchanged.
    Mock,
}

impl FlagKind {
    pub(crate) fn takes_value(self) -> bool {
        matches!(self, FlagKind::Text | FlagKind::Integer)
    }
}

impl Flag {
    pub(crate) fn matches(&self, text: &OsStr) -> bool {
        text == self.name
            || (self.kind == FlagKind::Mock
                && text
                    .as_encoded_bytes()
                    .strip_prefix(self.name.as_bytes())
                    .is_some_and(|rest| rest.starts_with(b"=")))
    }
}

/// Where a verb takes the instance's name: `name` on `plasmosome.*` verbs, `kernel` on the
/// others (spec 025 section 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceParam {
    Name,
    Kernel,
}

impl InstanceParam {
    pub(crate) fn key(self) -> &'static str {
        match self {
            InstanceParam::Name => "name",
            InstanceParam::Kernel => "kernel",
        }
    }
}

/// Where a parameter came from on the command line. A request line too large names the argument
/// its source gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgSource {
    /// One of the command's flags, by its name.
    Flag(&'static str),
    /// One of the command's positional arguments, by the name its help gives it.
    Positional(&'static str),
    /// The workload's argv after `--`.
    Argv,
}

/// One wire parameter, as the command line gave it.
#[derive(Debug, PartialEq, Eq)]
pub struct Param {
    pub key: &'static str,
    pub value: Json,
    pub source: ArgSource,
}

/// A verb that takes `deadline_ms`. `max_ms` is the verb's own maximum, if it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerbDeadline {
    pub max_ms: Option<u64>,
}

/// The one request a served command sends. The client adds the instance's name, from socket
/// selection, and `deadline_ms`, from the time left. Whether the request changes state is its
/// table entry's [`changes_state`](crate::Command::changes_state).
#[derive(Debug, PartialEq, Eq)]
pub struct VerbCall {
    pub method: &'static str,
    pub instance: InstanceParam,
    /// The parameters, which must not repeat a key, use the instance's key or `deadline_ms`, or
    /// hold an object that repeats a key.
    pub params: Vec<Param>,
    /// `None` for a verb that takes no `deadline_ms`.
    pub deadline: Option<VerbDeadline>,
}

/// A served command's own arguments, as the full parse read them: its flags, its positional
/// arguments and its argv. Only the parse makes one with anything in it. A command's build turns
/// them into its [`VerbCall`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandArgs {
    given: Vec<Given>,
    kernel_at: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Given {
    source: ArgSource,
    position: usize,
    argument: String,
    sent: Sent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Sent {
    Text(String),
    Number(String),
    True,
    Argv(Vec<String>),
}

impl CommandArgs {
    /// The parameter `flag` gives, sent as `flag.key`, or `None` when the line does not give it.
    /// A bare flag gives `true`, and an integer gives a number with the digits as written.
    pub fn flag(&self, flag: &Flag) -> Option<Param> {
        self.param(ArgSource::Flag(flag.name), flag.key)
    }

    /// The positional argument the command's help names `operand`, as a string sent as `key`.
    pub fn positional(&self, operand: &'static str, key: &'static str) -> Option<Param> {
        self.param(ArgSource::Positional(operand), key)
    }

    /// The workload's argv after `--`, as an array of strings sent as `key`, or `None` when the
    /// line has no `--`.
    pub fn argv(&self, key: &'static str) -> Option<Param> {
        self.param(ArgSource::Argv, key)
    }

    fn param(&self, source: ArgSource, key: &'static str) -> Option<Param> {
        let given = self.given.iter().find(|given| given.source == source)?;
        let value = match &given.sent {
            Sent::Text(text) => Json::String(text.clone()),
            Sent::Number(text) => Json::Number(
                Number::new(text.clone()).expect("the parse checked the integer's digits"),
            ),
            Sent::True => Json::Bool(true),
            Sent::Argv(argv) => Json::Array(argv.iter().cloned().map(Json::String).collect()),
        };
        Some(Param { key, value, source })
    }

    pub(crate) fn give(&mut self, source: ArgSource, position: usize, argument: &str, sent: Sent) {
        self.given.push(Given {
            source,
            position,
            argument: argument.to_string(),
            sent,
        });
    }

    pub(crate) fn kernel_at(&mut self, position: usize) {
        self.kernel_at = Some(position);
    }

    pub(crate) fn kernel_position(&self) -> Option<usize> {
        self.kernel_at
    }

    pub(crate) fn locate(&self, source: ArgSource) -> Option<(usize, &str)> {
        let given = self.given.iter().find(|given| given.source == source)?;
        Some((given.position, &given.argument))
    }
}

pub(crate) fn is_integer(text: &str) -> bool {
    text == "0"
        || (text.starts_with(|first: char| ('1'..='9').contains(&first))
            && text.bytes().all(|byte| byte.is_ascii_digit()))
}
