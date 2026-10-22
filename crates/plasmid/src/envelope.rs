use std::ffi::OsStr;

use crate::Exit;

#[derive(Clone, Copy)]
pub(crate) enum Form {
    Json,
    Human,
}

#[derive(Clone, Copy)]
enum Code {
    InvalidArgument,
    NotServed,
    LocalIo,
}

impl Code {
    fn as_str(self) -> &'static str {
        match self {
            Code::InvalidArgument => "invalid_argument",
            Code::NotServed => "not_served",
            Code::LocalIo => "local_io",
        }
    }
}

enum Detail {
    Text(String),
    Number(i32),
}

pub(crate) struct Refusal {
    command: String,
    code: Code,
    message: String,
    field: Option<(&'static str, Detail)>,
    form: Form,
}

impl Refusal {
    pub(crate) fn usage(message: String, argument: Option<&OsStr>, form: Form) -> Refusal {
        Refusal {
            command: "invalid".to_string(),
            code: Code::InvalidArgument,
            message,
            field: argument.map(|argument| {
                (
                    "argument",
                    Detail::Text(argument.to_string_lossy().into_owned()),
                )
            }),
            form,
        }
    }

    pub(crate) fn not_served(method: &str, message: String, form: Form) -> Refusal {
        Refusal {
            command: method.to_string(),
            code: Code::NotServed,
            message,
            field: Some(("method", Detail::Text(method.to_string()))),
            form,
        }
    }

    pub(crate) fn local_io(
        command: &str,
        message: String,
        errno: Option<i32>,
        form: Form,
    ) -> Refusal {
        Refusal {
            command: command.to_string(),
            code: Code::LocalIo,
            message,
            field: errno.map(|errno| ("errno", Detail::Number(errno))),
            form,
        }
    }

    pub(crate) fn exit(&self) -> Exit {
        match self.code {
            Code::InvalidArgument => Exit::Usage,
            Code::NotServed => Exit::Refused,
            Code::LocalIo => Exit::Unavailable,
        }
    }

    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        match self.form {
            Form::Json => {
                out.push_str("{\"schema\":1,\"command\":");
                write_string(&mut out, &self.command);
                out.push_str(",\"error\":{\"domain\":\"client\",\"code\":");
                write_string(&mut out, self.code.as_str());
                out.push_str(",\"message\":");
                write_string(&mut out, &self.message);
                if let Some((key, value)) = &self.field {
                    out.push(',');
                    write_string(&mut out, key);
                    out.push(':');
                    match value {
                        Detail::Text(text) => write_string(&mut out, text),
                        Detail::Number(number) => out.push_str(&number.to_string()),
                    }
                }
                out.push_str("}}");
            }
            Form::Human => {
                show(&mut out, &self.message);
                out.push_str("\n(client ");
                out.push_str(self.code.as_str());
                out.push(')');
            }
        }
        out.push('\n');
        out
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' | '\\' => {
                out.push('\\');
                out.push(character);
            }
            control if is_terminal_control(control) => {
                out.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn show(out: &mut String, text: &str) {
    for character in text.chars() {
        if is_terminal_control(character) {
            out.push_str(&format!("<U+{:04X}>", u32::from(character)));
        } else {
            out.push(character);
        }
    }
}

pub(crate) fn is_terminal_control(character: char) -> bool {
    matches!(
        u32::from(character),
        0x00..=0x1f | 0x7f..=0x9f | 0x202a..=0x202e | 0x2066..=0x2069
    )
}
