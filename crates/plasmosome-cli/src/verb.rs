use std::cmp::Reverse;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::args::{ArgSource, CommandArgs, Param, VerbCall};
use crate::cli::{Context, INVALID, Invocation, client, local_io, refuse};
use crate::clock::Deadline;
use crate::codes::{ClientCode, Exit};
use crate::env::Env;
use crate::envelope::{ClientError, EmitError, Envelope, ErrorBody, Form, Instance, Stream, emit};
use crate::exchange::{Unreplied, exchange_with};
use crate::json::{self, Json};
use crate::refusal::number;
use crate::reply::{Reply, cut_off, judge_reply, kernel_exit, unanswered};
use crate::select::{self, Named, Selected, Target, Unselected, VARIABLE};

const MAX_REQUEST_BYTES: usize = 1_048_576;

const SHORTEST_NAME: &str = "x";

pub(crate) fn run(
    call: &VerbCall,
    invocation: Invocation,
    context: &Context<'_>,
    start: Instant,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let Invocation {
        command,
        kernel,
        socket,
        deadline_ms: total_ms,
        form,
        args,
    } = invocation;
    if repeats_a_key(call) {
        let error = client(
            ClientCode::Internal,
            format!("{}'s call repeats a key", command.command),
            Vec::new(),
        );
        return refuse(command.command.to_string(), error, form, stdout, stderr);
    }
    let total_ms = total_ms.unwrap_or(command.default_deadline_ms);
    let total = Duration::from_millis(u64::from(total_ms));
    let given = early(&kernel, &socket, context.env, &args);
    let over = measure(call, &args, given, total);
    if let Some(Over {
        bytes,
        culprit: Culprit::Param(index),
    }) = over
    {
        let instance = socket.map(|socket| Instance {
            name: kernel,
            socket,
        });
        let (subject, argument) = described(&call.params[index], &args);
        return too_large(&subject, bytes, argument, instance, form, stdout, stderr);
    }
    let target = match select::target(socket, kernel, context.env) {
        Ok(target) => target,
        Err(error) => {
            let named = if error.code() == ClientCode::InvalidArgument {
                INVALID
            } else {
                command.command
            };
            return refuse(named.to_string(), error, form, stdout, stderr);
        }
    };
    let deadline = Deadline::new(context.clock, start, total_ms);
    let changes_state = command.changes_state;
    let uid = context.trusted_uid;
    let (instance, outcome) = match target {
        Target::Named(named) => {
            if let Some(Over { bytes, .. }) = over {
                return name_too_large(bytes, named, form, stdout, stderr);
            }
            let socket = Path::new(&named.instance.socket);
            let name = named.instance.name.as_deref();
            let outcome = select::connect(socket, uid, named.origin)
                .map_err(ErrorBody::Client)
                .and_then(|stream| {
                    answer(call, changes_state, stream, socket, name, uid, &deadline)
                });
            (Some(named.instance), outcome)
        }
        Target::Default(instances) => {
            let chosen = select::select_default(&instances, uid);
            if let Ok(instance) = &chosen
                && let Some(Over {
                    bytes,
                    culprit: Culprit::Param(index),
                }) = measure(
                    call,
                    &args,
                    instance.name.clone().map(Name::not_given),
                    total,
                )
            {
                let (subject, argument) = described(&call.params[index], &args);
                return too_large(&subject, bytes, argument, chosen.ok(), form, stdout, stderr);
            }
            match chosen.and_then(select::connect_chosen) {
                Ok(Selected { stream, instance }) => {
                    let socket = Path::new(&instance.socket);
                    let name = instance.name.as_deref();
                    let outcome = answer(call, changes_state, stream, socket, name, uid, &deadline);
                    (Some(instance), outcome)
                }
                Err(Unselected { error, instance }) => (instance, Err(ErrorBody::Client(error))),
            }
        }
    };
    let exit = match &outcome {
        Ok(_) => Exit::Done,
        Err(ErrorBody::Kernel(error)) => kernel_exit(error),
        Err(ErrorBody::Client(error)) => error.code().exit(),
    };
    let envelope = Envelope {
        command: command.command.to_string(),
        instance,
        outcome,
    };
    let Err(EmitError {
        stream: Stream::Stdout,
        error,
    }) = emit(&envelope, form, stdout, stderr)
    else {
        return exit;
    };
    let lost = if changes_state {
        lost_answer(&error)
    } else {
        local_io(&error)
    };
    let exit = lost.code().exit();
    let failed = Envelope {
        outcome: Err(ErrorBody::Client(lost)),
        ..envelope
    };
    let _ = emit(&failed, form, stdout, stderr);
    exit
}

fn repeats_a_key(call: &VerbCall) -> bool {
    call.params.iter().enumerate().any(|(index, param)| {
        param.key == call.instance.key()
            || param.key == "deadline_ms"
            || call.params[..index]
                .iter()
                .any(|before| before.key == param.key)
            || json::repeats_a_key(&param.value)
    })
}

struct Name {
    text: String,
    given_at: Option<usize>,
}

impl Name {
    fn not_given(text: String) -> Name {
        Name {
            text,
            given_at: None,
        }
    }
}

fn early(
    kernel: &Option<String>,
    socket: &Option<String>,
    env: &dyn Env,
    args: &CommandArgs,
) -> Option<Name> {
    let given = |text: String, at: Option<usize>| Name {
        text,
        given_at: Some(at.unwrap_or(usize::MAX)),
    };
    match (kernel, socket) {
        (Some(name), _) => Some(given(name.clone(), args.kernel_position())),
        (None, Some(_)) => None,
        (None, None) => Some(match select::variable(env) {
            Ok(Some(name)) => given(name, None),
            _ => Name::not_given(SHORTEST_NAME.to_string()),
        }),
    }
}

enum Culprit {
    Name,
    Param(usize),
}

struct Over {
    bytes: usize,
    culprit: Culprit,
}

fn measure(
    call: &VerbCall,
    args: &CommandArgs,
    name: Option<Name>,
    total: Duration,
) -> Option<Over> {
    let widest = call.deadline.map(|verb| deadline_ms(total, verb.max_ms));
    let shown = name.as_ref().map(|name| name.text.as_str());
    let bytes = request(call, shown, widest).len() - 1;
    if bytes <= MAX_REQUEST_BYTES {
        return None;
    }
    let params = call.params.iter().enumerate().map(|(index, param)| {
        let at = args.locate(param.source).map_or(usize::MAX, |(at, _)| at);
        (written(&param.value), at, Culprit::Param(index))
    });
    let named = name.and_then(|Name { text, given_at }| {
        given_at.map(|at| (written(&Json::String(text)), at, Culprit::Name))
    });
    let (_, _, culprit) = params
        .chain(named)
        .min_by_key(|(written, at, _)| (Reverse(*written), *at))?;
    Some(Over { bytes, culprit })
}

fn written(value: &Json) -> usize {
    json::write_compact(value).len()
}

fn described(param: &Param, args: &CommandArgs) -> (String, String) {
    let (subject, label) = match param.source {
        ArgSource::Flag(flag) => (format!("{flag}'s value"), flag),
        ArgSource::Positional(operand) => (operand.to_string(), operand),
        ArgSource::Argv => ("the argv after --".to_string(), "--"),
    };
    let argument = args
        .locate(param.source)
        .map_or(label, |(_, argument)| argument);
    (subject, argument.to_string())
}

fn request(call: &VerbCall, name: Option<&str>, deadline_ms: Option<u64>) -> String {
    let name = name.map(|name| Json::String(name.to_string()));
    let deadline_ms = deadline_ms.map(number);
    let members = name
        .iter()
        .map(|name| (call.instance.key(), name))
        .chain(call.params.iter().map(|param| (param.key, &param.value)))
        .chain(deadline_ms.iter().map(|ms| ("deadline_ms", ms)));
    let method = json::write_compact(&Json::String(call.method.to_string()));
    let mut line = format!("{{\"id\":1,\"method\":{method},\"params\":{{");
    for (index, (key, value)) in members.enumerate() {
        if index > 0 {
            line.push(',');
        }
        line.push_str(&json::write_compact(&Json::String(key.to_string())));
        line.push(':');
        line.push_str(&json::write_compact(value));
    }
    line.push_str("}}\n");
    line
}

fn name_too_large(
    bytes: usize,
    named: Named,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let (source, argument) = if named.from_variable {
        (VARIABLE, VARIABLE.to_string())
    } else {
        ("--kernel", named.instance.name.clone().unwrap_or_default())
    };
    let subject = format!("{source}'s name");
    let instance = Some(named.instance);
    too_large(&subject, bytes, argument, instance, form, stdout, stderr)
}

fn too_large(
    subject: &str,
    bytes: usize,
    argument: String,
    instance: Option<Instance>,
    form: Form,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let error = client(
        ClientCode::InvalidArgument,
        format!(
            "{subject} makes the request line {bytes} bytes before its newline, and a daemon \
             reads at most {MAX_REQUEST_BYTES}"
        ),
        vec![("argument".to_string(), Json::String(argument))],
    );
    let exit = error.code().exit();
    let envelope = Envelope {
        command: INVALID.to_string(),
        instance,
        outcome: Err(ErrorBody::Client(error)),
    };
    let _ = emit(&envelope, form, stdout, stderr);
    exit
}

fn answer(
    call: &VerbCall,
    changes_state: bool,
    stream: UnixStream,
    socket: &Path,
    name: Option<&str>,
    trusted_uid: u32,
    deadline: &Deadline<'_>,
) -> Result<Json, ErrorBody> {
    let line = exchange_with(stream, socket, trusted_uid, deadline, || {
        let left = call
            .deadline
            .map(|verb| deadline_ms(deadline.left(), verb.max_ms));
        request(call, name, left)
    })
    .map_err(|unreplied| ErrorBody::Client(unreplied_error(unreplied, changes_state)))?;
    match judge_reply(&line).map_err(ErrorBody::Client)? {
        Reply::Result(result) => Ok(result),
        Reply::Kernel(error) => Err(ErrorBody::Kernel(error)),
    }
}

fn unreplied_error(unreplied: Unreplied, changes_state: bool) -> ClientError {
    match unreplied {
        Unreplied::NotSent(error) => error,
        Unreplied::Unanswered(failure) => unanswered(failure, changes_state),
        Unreplied::CutOff(error) => cut_off(&error, changes_state),
    }
}

fn deadline_ms(left: Duration, max_ms: Option<u64>) -> u64 {
    let left = u64::try_from(left.as_millis()).unwrap_or(u64::MAX);
    let margin = (left / 10).min(1_000);
    (left - margin).min(max_ms.unwrap_or(u64::MAX)).max(1)
}

fn lost_answer(error: &io::Error) -> ClientError {
    client(
        ClientCode::OutcomeUnknown,
        format!(
            "the daemon answered, but its answer could not be written to stdout: {error}; observe \
             before repeating the request"
        ),
        vec![("cause".to_string(), Json::String("local_io".to_string()))],
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use plasmosome_core::private_socket::effective_uid;

    use super::*;
    use crate::args::{InstanceParam, VerbDeadline};
    use crate::cli::is_instance_name;
    use crate::clock::{Clock, SystemClock};
    use crate::exchange::{Transport, converse};
    use crate::json::Style;

    const EPIPE: i32 = 32;

    #[test]
    fn the_verb_deadline_is_the_time_left_less_its_margin() {
        let ms = |left: u64, max_ms| deadline_ms(Duration::from_millis(left), max_ms);
        assert_eq!(ms(10_000, None), 9_000);
        assert_eq!(ms(300, None), 270);
        assert_eq!(ms(9, None), 9);
        assert_eq!(ms(1, None), 1);
        assert_eq!(ms(0, None), 1);
        assert_eq!(ms(3_600_000, None), 3_599_000);
        assert_eq!(ms(3_600_000, Some(600_000)), 600_000);
        assert_eq!(ms(500, Some(600_000)), 450);
        assert_eq!(deadline_ms(Duration::from_micros(1_999), None), 1);
    }

    #[test]
    fn the_stand_in_writes_no_more_than_any_name() {
        assert!(is_instance_name(SHORTEST_NAME) && !is_instance_name(""));
        assert_eq!(written(&Json::String(SHORTEST_NAME.to_string())), 3);
        let mut out = String::new();
        for one in (0..=0x10_FFFF).filter_map(char::from_u32) {
            out.clear();
            json::write_string(&mut out, one.encode_utf8(&mut [0; 4]), Style::Compact);
            assert!(out.len() - 2 >= one.len_utf8(), "U+{:04X}", u32::from(one));
        }
    }

    struct Counted(AtomicUsize);

    impl Clock for Counted {
        fn now(&self) -> Instant {
            self.0.fetch_add(1, Ordering::Relaxed);
            Instant::now()
        }
    }

    #[test]
    fn the_time_left_is_not_read_before_the_peer_check() {
        let call = VerbCall {
            method: "exec.status",
            instance: InstanceParam::Kernel,
            params: Vec::new(),
            deadline: Some(VerbDeadline { max_ms: None }),
        };
        let socket = Path::new("/x/control.uds");
        for (trusted_uid, peer_checked) in [(effective_uid() + 1, false), (effective_uid(), true)] {
            let clock = Counted(AtomicUsize::new(0));
            let deadline = Deadline::new(&clock, Instant::now(), 10_000);
            let (ours, mut theirs) = UnixStream::pair().expect("a socket pair");
            theirs
                .write_all(b"{\"id\":1,\"result\":{}}\n")
                .expect("the answer waits in the pair");
            let answered = answer(&call, true, ours, socket, None, trusted_uid, &deadline);
            let refused = matches!(
                &answered,
                Err(ErrorBody::Client(error)) if error.code() == ClientCode::PeerMismatch
            );
            assert_eq!(refused, !peer_checked, "{answered:?}");
            let reads = clock.0.load(Ordering::Relaxed);
            assert_eq!(reads > 0, peer_checked, "the clock was read {reads} times");
        }
    }

    struct TakesThen {
        takes: usize,
        taken: Cell<usize>,
        refuses: Option<i32>,
        writes: Cell<usize>,
    }

    impl Transport for TakesThen {
        fn bound(&self, _: Duration, _: bool) -> io::Result<()> {
            Ok(())
        }

        fn read_some(&self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }

        fn write_some(&self, bytes: &[u8]) -> io::Result<usize> {
            self.writes.set(self.writes.get() + 1);
            let room = self.takes - self.taken.get();
            if room == 0 {
                return self
                    .refuses
                    .map_or(Ok(0), |errno| Err(io::Error::from_raw_os_error(errno)));
            }
            let count = room.min(bytes.len());
            self.taken.set(self.taken.get() + count);
            Ok(count)
        }
    }

    fn failed_after(takes: usize, refuses: Option<i32>, changes_state: bool) -> ClientError {
        let transport = TakesThen {
            takes,
            taken: Cell::new(0),
            refuses,
            writes: Cell::new(0),
        };
        let deadline = Deadline::new(&SystemClock, Instant::now(), 10_000);
        let request = b"{\"id\":1,\"method\":\"cell.kill\",\"params\":{}}\n";
        let unreplied = converse(&transport, Path::new("/x/control.uds"), request, &deadline)
            .expect_err("a write that fails gives no reply");
        assert_eq!(transport.taken.get(), takes);
        assert_eq!(transport.writes.get(), usize::from(takes > 0) + 1);
        unreplied_error(unreplied, changes_state)
    }

    fn cause(error: &ClientError) -> Option<&str> {
        match json::member(error.fields(), "cause") {
            Some(Json::String(cause)) => Some(cause),
            _ => None,
        }
    }

    #[test]
    fn a_write_that_fails_or_takes_nothing() {
        for refuses in [Some(EPIPE), None] {
            let error = failed_after(10, refuses, true);
            assert_eq!(error.code(), ClientCode::OutcomeUnknown, "{refuses:?}");
            assert_eq!(error.code().exit(), Exit::OutcomeUnknown);
            assert_eq!(cause(&error), Some("transport"), "{refuses:?}");
            let error = failed_after(10, refuses, false);
            assert_eq!(error.code(), ClientCode::NoReply, "{refuses:?}");
            assert_eq!(error.code().exit(), Exit::Unavailable);
            assert_eq!(cause(&error), Some("transport"), "{refuses:?}");
            for changes_state in [true, false] {
                let error = failed_after(0, refuses, changes_state);
                let case = format!("{refuses:?}, {changes_state}");
                assert_eq!(error.code(), ClientCode::Transport, "{case}");
                assert_eq!(error.code().exit(), Exit::Unavailable);
                assert_eq!(
                    json::member(error.fields(), "errno"),
                    refuses.map(number).as_ref(),
                    "{case}"
                );
            }
        }
    }
}
