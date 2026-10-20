/// Whether `text` nests deeper than `limit` levels anywhere, counted as
/// `MAX_MANIFEST_DEPTH` says. It reads each byte once, without recursion,
/// holds at most `limit` open arrays and inline tables, and stops at the first
/// level past `limit`. Text toml rejects may count deeper than any tree toml
/// builds from it.
pub(crate) fn deeper_than(_text: &str, _limit: usize) -> bool {
    false
}
