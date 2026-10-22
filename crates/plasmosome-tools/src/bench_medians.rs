use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

/// The benchmarks the summary lists, in its order. Each is a criterion id, the directory under
/// criterion's output directory that holds its `new/estimates.json`.
pub const BENCHMARKS: [&str; 8] = [
    "attach_detach",
    "ledger_replay/10",
    "ledger_replay/100",
    "ledger_replay/1000",
    "manifest_parse",
    "reconciler_step",
    "session_log_append",
    "session_log_read",
];

/// The most bytes read from one `estimates.json`. A longer file is refused.
pub const ESTIMATES_LIMIT: u64 = 1024 * 1024;

const HEADING: &str =
    "## Kernel benchmark medians\n\nFakeBackend timings exclude OS enforcement.\n\n";

/// Writes the median of each benchmark in [`BENCHMARKS`] to `out` as Markdown, then appends the
/// same text to `summary`.
///
/// `args` are the arguments after the program name: exactly one, criterion's output directory,
/// such as `target/criterion`. `summary` is the file `GITHUB_STEP_SUMMARY` names, or `None` when
/// that variable is unset.
///
/// Returns 0 when both writes succeed. Returns 1 when a median cannot be read, and then writes
/// nothing, or when the summary cannot be written. Returns 2 when `args` is not one argument.
/// Each failure is one line on `err`.
pub fn run(
    args: &[OsString],
    summary: Option<&Path>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let [root] = args else {
        return fail(
            err,
            2,
            "expected one argument, criterion's output directory, such as target/criterion",
        );
    };
    let text = match medians(Path::new(root)) {
        Ok(text) => text,
        Err(reason) => return fail(err, 1, &reason),
    };
    if let Err(error) = out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        return fail(err, 1, &format!("cannot write to stdout: {error}"));
    }
    let Some(summary) = summary else {
        return fail(err, 1, "GITHUB_STEP_SUMMARY is not set");
    };
    match append(summary, &text) {
        Ok(()) => 0,
        Err(error) => fail(
            err,
            1,
            &format!("cannot append to {}: {error}", summary.display()),
        ),
    }
}

fn medians(root: &Path) -> Result<String, String> {
    let mut text = String::from(HEADING);
    for name in BENCHMARKS {
        let median = median(&root.join(name).join("new").join("estimates.json"))?;
        text.push_str(&format!("- `{name}` {median:.6} ns\n"));
    }
    Ok(text)
}

fn median(path: &Path) -> Result<f64, String> {
    let shown = path.display();
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(ESTIMATES_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("cannot read {shown}: {error}"))?;
    if bytes.len() as u64 > ESTIMATES_LIMIT {
        return Err(format!("{shown} is over {ESTIMATES_LIMIT} bytes"));
    }
    let estimates: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("{shown} is not JSON: {error}"))?;
    estimates
        .pointer("/median/point_estimate")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| format!("{shown} has no number at median.point_estimate"))
}

fn append(path: &Path, text: &str) -> std::io::Result<()> {
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?
        .write_all(text.as_bytes())
}

fn fail(err: &mut dyn Write, code: u8, reason: &str) -> u8 {
    let _ = writeln!(err, "bench-medians: {reason}");
    code
}
