use crate::envelope::is_terminal_control;

pub(crate) const MAX_DEADLINE_MS: u32 = 3_600_000;

const MAX_DEADLINE_DIGITS: usize = 7;

pub(crate) fn is_instance_name(text: &str) -> bool {
    !matches!(text, "" | "." | "..")
        && !text.chars().any(|character| {
            matches!(character, '/' | '\\' | '\0') || is_terminal_control(character)
        })
}

pub(crate) fn is_normal_absolute(path: &str) -> bool {
    path.strip_prefix('/').is_some_and(|rest| {
        rest.split('/')
            .all(|component| !matches!(component, "" | "." | ".."))
    })
}

pub(crate) fn is_deadline_ms(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_DEADLINE_DIGITS
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && text
            .parse::<u32>()
            .is_ok_and(|deadline_ms| (1..=MAX_DEADLINE_MS).contains(&deadline_ms))
}
