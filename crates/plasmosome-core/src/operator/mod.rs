#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "plasmosome-9e4m's selection and plasmosome-0xy2's resolution are the first callers"
    )
)]
pub(crate) mod file;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "plasmosome-0xy2's resolution is the first caller")
)]
pub(crate) mod sources;
pub mod trusted;
