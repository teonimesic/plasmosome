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

fn is_terminal_control(character: char) -> bool {
    matches!(
        u32::from(character),
        0x00..=0x1f | 0x7f..=0x9f | 0x061c | 0x200e..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{is_terminal_control, show, write_string};

    const SPEC_025: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/specs/025-plasmosome-command-line.md"
    );

    const TERMINAL_CONTROLS_PARAGRAPH: &str = "**Terminal controls are escaped in both forms.**";

    const LIST_START: &str = "The terminal controls are ";

    fn named_code_points(text: &str) -> impl Iterator<Item = (Option<u32>, &str)> {
        text.split("U+").skip(1).map(|piece| {
            let rest =
                piece.trim_start_matches(|c: char| c.is_ascii_digit() || ('A'..='F').contains(&c));
            let digits = &piece[..piece.len() - rest.len()];
            let point = (4..=6)
                .contains(&digits.len())
                .then(|| u32::from_str_radix(digits, 16).expect("upper-case hex digits"));
            (point, rest)
        })
    }

    fn spec_025_terminal_controls() -> Vec<u32> {
        let spec = fs::read_to_string(SPEC_025).expect("spec 025 is readable beside the crate");
        let paragraph = spec
            .lines()
            .skip_while(|line| !line.starts_with(TERMINAL_CONTROLS_PARAGRAPH))
            .take_while(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let list = paragraph
            .split_once(LIST_START)
            .and_then(|(_, rest)| rest.split_once('.'))
            .map(|(list, _)| list)
            .unwrap_or_else(|| {
                panic!(
                    "spec 025's list of terminal controls was not found: no paragraph starts with {TERMINAL_CONTROLS_PARAGRAPH:?} and holds {LIST_START:?} and a period"
                )
            });
        let mut points = Vec::new();
        let mut range_open = false;
        for (point, rest) in named_code_points(list) {
            match point {
                Some(point) if range_open => {
                    let first = points.pop().expect("a range starts at a code point");
                    points.extend(first..=point);
                }
                Some(point) => points.push(point),
                None => {}
            }
            range_open = point.is_some() && rest.trim() == "to";
        }
        points.sort_unstable();
        points.dedup();
        if let Some(point) = named_code_points(&paragraph)
            .filter_map(|(point, _)| point)
            .find(|point| points.binary_search(point).is_err())
        {
            panic!(
                "U+{point:04X} is named in spec 025's paragraph {TERMINAL_CONTROLS_PARAGRAPH:?} but not in its list of terminal controls"
            );
        }
        points
    }

    fn assert_same_code_points(spec: &[u32], other: &[u32], other_name: &str) {
        let first = spec
            .iter()
            .chain(other)
            .copied()
            .filter(|point| spec.contains(point) != other.contains(point))
            .min();
        if let Some(point) = first {
            let (holder, lacking) = if spec.contains(&point) {
                ("spec 025's list", other_name)
            } else {
                (other_name, "spec 025's list")
            };
            panic!("U+{point:04X} is in {holder} but not in {lacking}");
        }
    }

    fn json_escape(rest: &str, point: u32) -> Option<&str> {
        let hex = rest.strip_prefix(char::from(0x5c_u8))?.strip_prefix('u')?;
        let digits = hex.get(..4)?;
        if !digits.chars().all(|c| c.is_ascii_hexdigit())
            || u32::from_str_radix(digits, 16) != Ok(point)
        {
            return None;
        }
        hex.get(4..)
    }

    fn code_point_shown(rest: &str, point: u32) -> Option<&str> {
        rest.strip_prefix(format!("<U+{point:04X}>").as_str())
    }

    fn assert_written_as_spec_025_says(
        writer: &str,
        written: &str,
        text: &str,
        listed: &[u32],
        escape: fn(&str, u32) -> Option<&str>,
    ) {
        let mut rest = written;
        for character in text.chars() {
            let point = u32::from(character);
            let is_listed = listed.binary_search(&point).is_ok();
            let after = if is_listed {
                escape(rest, point)
            } else {
                rest.strip_prefix(character)
            };
            rest = after.unwrap_or_else(|| {
                let found: String = rest.chars().take(8).collect();
                if is_listed {
                    panic!("U+{point:04X} is listed, so {writer} escapes it, not as {found:?}")
                }
                panic!("U+{point:04X} is not listed, so {writer} writes it raw, not as {found:?}")
            });
        }
        assert!(
            rest.is_empty(),
            "{writer} writes {rest:?} after the last character"
        );
    }

    #[test]
    fn both_writers_follow_spec_025s_list_for_every_character() {
        let listed = spec_025_terminal_controls();
        let escaped: Vec<u32> = (0..=0x10ffff)
            .filter(|&point| char::from_u32(point).is_some_and(is_terminal_control))
            .collect();
        assert_same_code_points(&listed, &escaped, "is_terminal_control's list");
        assert_eq!(listed.len(), 77, "spec 025 section 3 names 77 code points");
        let every: String = (0..=0x10ffff).filter_map(char::from_u32).collect();
        let mut shown = String::new();
        show(&mut shown, &every);
        assert_written_as_spec_025_says("show", &shown, &every, &listed, code_point_shown);
        let backslash = char::from(0x5c_u8);
        let text: String = every
            .chars()
            .filter(|&character| character != '"' && character != backslash)
            .collect();
        let mut written = String::new();
        write_string(&mut written, &text);
        let body = written
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("write_string writes a string between quotes");
        assert_written_as_spec_025_says("write_string", body, &text, &listed, json_escape);
    }
}
