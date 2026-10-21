#[cfg_attr(
    not(test),
    expect(dead_code, reason = "plasmosome-0xy2's resolution is the first caller")
)]
pub(crate) mod sources;
pub mod trusted;
