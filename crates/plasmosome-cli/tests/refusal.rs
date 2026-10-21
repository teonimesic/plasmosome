use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use plasmosome_cli::json::{Json, Number};
use plasmosome_cli::{ClientCode, ClientError, Exit, PathOrigin, map_refusal};
use plasmosome_core::private_socket::{PrivateSocketError, effective_uid};

const SPEC_025: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/025-plasmosome-command-line.md"
);

const REFUSAL_TABLE_HEADER: &str = "| #124 error | Client code | `rule` | Exit |";

const SOCKET: &str = "/Users/o/.plasmosome/instances/a/control.uds";

const CHECKED: &str = "/Users/o/.plasmosome/instances/checked/control.uds";

const DIRECTORY: &str = "/Users/o/.plasmosome/instances/a";

const INSTANCES: &str = "/Users/o/.plasmosome/instances";

const TRUSTED: u32 = 501;

fn text(value: &str) -> Json {
    Json::String(value.to_string())
}

fn number(value: impl ToString) -> Json {
    Json::Number(Number::new(value.to_string()).expect("the fixture is a JSON number"))
}

fn at(path: &str) -> PathBuf {
    PathBuf::from(path)
}

fn fields(pairs: Vec<(&str, Json)>) -> Vec<(String, Json)> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn field<'a>(error: &'a ClientError, key: &str) -> Option<&'a Json> {
    error
        .fields()
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn from_socket(error: &PrivateSocketError) -> ClientError {
    map_refusal(error, Path::new(CHECKED), PathOrigin::SocketFlag, TRUSTED)
}

fn from_home(error: &PrivateSocketError) -> ClientError {
    map_refusal(error, Path::new(CHECKED), PathOrigin::Home, TRUSTED)
}

fn variant_name(error: &PrivateSocketError) -> String {
    format!("{error:?}")
        .split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_string()
}

struct Row {
    error: PrivateSocketError,
    code: ClientCode,
    exit: u8,
    fields: Vec<(String, Json)>,
}

fn row(error: PrivateSocketError, code: ClientCode, exit: u8, pairs: Vec<(&str, Json)>) -> Row {
    Row {
        error,
        code,
        exit,
        fields: fields(pairs),
    }
}

fn unsafe_socket(error: PrivateSocketError, path: &str, rule: &str) -> Row {
    row(
        error,
        ClientCode::UnsafeSocket,
        2,
        vec![("path", text(path)), ("rule", text(rule))],
    )
}

fn internal(error: PrivateSocketError) -> Row {
    row(error, ClientCode::Internal, 70, Vec::new())
}

macro_rules! every_variant {
    ($($variant:ident => $row:expr),* $(,)?) => {
        const LISTED: &[&str] = &[$(stringify!($variant)),*];

        fn every_row_from_socket() -> Vec<Row> {
            vec![$($row),*]
        }

        fn position(error: &PrivateSocketError) -> usize {
            let name = match error {
                $(PrivateSocketError::$variant { .. } => stringify!($variant),)*
            };
            LISTED
                .iter()
                .position(|listed| *listed == name)
                .expect("each variant is listed")
        }
    };
}

every_variant! {
    BadPath => row(
        PrivateSocketError::BadPath {
            path: at("/Users/o/../a/control.uds"),
        },
        ClientCode::InvalidArgument,
        64,
        vec![("argument", text("--socket"))],
    ),
    SymlinkInPath => unsafe_socket(
        PrivateSocketError::SymlinkInPath { path: at("/tmp") },
        "/tmp",
        "symlink",
    ),
    NoDirectory => row(
        PrivateSocketError::NoDirectory {
            path: at(DIRECTORY),
        },
        ClientCode::NoInstance,
        1,
        vec![("path", text(DIRECTORY))],
    ),
    NoSocket => row(
        PrivateSocketError::NoSocket { path: at(SOCKET) },
        ClientCode::NotRunning,
        1,
        vec![("path", text(SOCKET))],
    ),
    NotDirectory => unsafe_socket(
        PrivateSocketError::NotDirectory {
            path: at(DIRECTORY),
        },
        DIRECTORY,
        "not_directory",
    ),
    ForeignOwner => unsafe_socket(
        PrivateSocketError::ForeignOwner {
            path: at("/Users/o"),
            uid: 502,
        },
        "/Users/o",
        "owner",
    ),
    Replaceable => unsafe_socket(
        PrivateSocketError::Replaceable {
            path: at("/Users/o/.plasmosome"),
            mode: 0o777,
        },
        "/Users/o/.plasmosome",
        "mode",
    ),
    ReplaceableByAcl => unsafe_socket(
        PrivateSocketError::ReplaceableByAcl {
            path: at("/Users/o/.plasmosome"),
        },
        "/Users/o/.plasmosome",
        "acl",
    ),
    OwnershipIgnored => unsafe_socket(
        PrivateSocketError::OwnershipIgnored {
            path: at("/Volumes/u"),
        },
        "/Volumes/u",
        "owner",
    ),
    NotPrivate => row(
        PrivateSocketError::NotPrivate {
            path: at(DIRECTORY),
            mode: 0o750,
        },
        ClientCode::UnsafeSocket,
        2,
        vec![
            ("path", text(DIRECTORY)),
            ("rule", text("mode")),
            ("fix", text(&format!("chmod 700 '{DIRECTORY}'"))),
        ],
    ),
    AclPresent => unsafe_socket(
        PrivateSocketError::AclPresent {
            path: at(DIRECTORY),
        },
        DIRECTORY,
        "acl",
    ),
    Replaced => internal(PrivateSocketError::Replaced {
        path: at(DIRECTORY),
    }),
    BadName => unsafe_socket(
        PrivateSocketError::BadName {
            name: "a/b".to_string(),
        },
        CHECKED,
        "name",
    ),
    PathTooLong => row(
        PrivateSocketError::PathTooLong {
            path: at(SOCKET),
            max: 103,
        },
        ClientCode::SocketPathTooLong,
        2,
        vec![("path", text(SOCKET)), ("max_bytes", number(103))],
    ),
    AddressInUse => internal(PrivateSocketError::AddressInUse { path: at(SOCKET) }),
    NotASocket => unsafe_socket(
        PrivateSocketError::NotASocket { path: at(SOCKET) },
        SOCKET,
        "not_socket",
    ),
    SocketOwner => unsafe_socket(
        PrivateSocketError::SocketOwner {
            path: at(SOCKET),
            uid: 502,
        },
        SOCKET,
        "owner",
    ),
    SocketMode => unsafe_socket(
        PrivateSocketError::SocketMode {
            path: at(SOCKET),
            mode: 0o666,
        },
        SOCKET,
        "mode",
    ),
    BindEscaped => internal(PrivateSocketError::BindEscaped { path: at(SOCKET) }),
    PeerCredentials => row(
        PrivateSocketError::PeerCredentials { errno: 57 },
        ClientCode::PeerMismatch,
        2,
        vec![("path", text(CHECKED)), ("uid", number(TRUSTED))],
    ),
    PeerMismatch => row(
        PrivateSocketError::PeerMismatch {
            trusted: TRUSTED,
            found: 65,
        },
        ClientCode::PeerMismatch,
        2,
        vec![
            ("path", text(CHECKED)),
            ("uid", number(TRUSTED)),
            ("peer_uid", number(65)),
        ],
    ),
    Io => row(
        PrivateSocketError::Io {
            op: "openat",
            path: at(DIRECTORY),
            errno: 24,
        },
        ClientCode::LocalIo,
        1,
        vec![("path", text(DIRECTORY)), ("errno", number(24))],
    ),
}

#[test]
fn every_variant_has_its_row() {
    let rows = every_row_from_socket();
    assert_eq!(rows.len(), 22, "PrivateSocketError has 22 variants");
    for row in rows {
        let refusal = from_socket(&row.error);
        let name = variant_name(&row.error);
        assert_eq!(refusal.code(), row.code, "{name}'s code");
        assert_eq!(refusal.code().exit().code(), row.exit, "{name}'s exit");
        assert_eq!(refusal.fields(), row.fields.as_slice(), "{name}'s fields");
        assert!(!refusal.message().is_empty(), "{name} has no message");
    }
}

#[test]
fn only_bad_path_reads_its_origin() {
    for row in every_row_from_socket() {
        let name = variant_name(&row.error);
        let same = from_socket(&row.error) == from_home(&row.error);
        assert_eq!(same, name != "BadPath", "{name} from $HOME");
    }
}

fn spec_025_refusal_rows() -> Vec<[String; 4]> {
    let spec = fs::read_to_string(SPEC_025).expect("spec 025 is readable beside the crate");
    spec.lines()
        .skip_while(|line| *line != REFUSAL_TABLE_HEADER)
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let variant = cells[1].split('`').nth(1).unwrap_or_default();
            [
                variant.to_string(),
                cells[2].to_string(),
                cells[3].to_string(),
                cells[4].to_string(),
            ]
        })
        .collect()
}

fn as_spec_row(error: &PrivateSocketError) -> [String; 4] {
    let flag = from_socket(error);
    let home = from_home(error);
    let (code, exit) = if flag.code() == home.code() {
        (
            format!("`{}`", flag.code().as_str()),
            flag.code().exit().code().to_string(),
        )
    } else {
        (
            format!(
                "`{}` from `--socket`; `{}` from `$HOME`",
                flag.code().as_str(),
                home.code().as_str()
            ),
            format!(
                "{} or {}",
                flag.code().exit().code(),
                home.code().exit().code()
            ),
        )
    };
    let rule = match field(&flag, "rule") {
        Some(Json::String(rule)) => format!("`{rule}`"),
        _ => String::new(),
    };
    [variant_name(error), code, rule, exit]
}

#[test]
fn every_variant_is_listed_once_and_has_a_row_in_spec_025() {
    let rows = every_row_from_socket();
    let positions: Vec<usize> = rows.iter().map(|row| position(&row.error)).collect();
    assert_eq!(
        positions,
        (0..rows.len()).collect::<Vec<usize>>(),
        "each variant once, in the enum's order"
    );
    let table = spec_025_refusal_rows();
    for row in &rows {
        let name = variant_name(&row.error);
        let listed = table
            .get(position(&row.error))
            .map(|cells| cells[0].as_str());
        assert_eq!(listed, Some(name.as_str()), "spec 025's row for {name}");
    }
}

#[test]
fn the_rows_are_spec_025s() {
    let table = spec_025_refusal_rows();
    assert_eq!(
        table.len(),
        22,
        "spec 025's table of #124's errors was not found whole"
    );
    let mapped: Vec<[String; 4]> = every_row_from_socket()
        .iter()
        .map(|row| as_spec_row(&row.error))
        .collect();
    assert_eq!(mapped, table);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!(
            "plasmosome-cli-refusal-{}-{name}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("the scratch directory is new");
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("the directory exists")
        .permissions()
        .mode()
        & 0o7777
}

#[test]
fn the_fix_survives_a_shell() {
    let scratch = Scratch::new("fix");
    let directory = scratch.0.join(r#"it's a "dir" $x"#);
    fs::create_dir(&directory).expect("the directory is new");
    fs::set_permissions(&directory, Permissions::from_mode(0o750))
        .expect("the directory takes mode 0750");
    assert_eq!(mode_of(&directory), 0o750);
    let error = PrivateSocketError::NotPrivate {
        path: directory.clone(),
        mode: 0o750,
    };
    let refusal = from_socket(&error);
    let Some(Json::String(fix)) = field(&refusal, "fix") else {
        panic!("NotPrivate carries no fix: {refusal:?}");
    };
    let status = Command::new("sh")
        .arg("-c")
        .arg(fix)
        .env_remove("x")
        .status()
        .expect("sh runs");
    assert!(status.success(), "{fix} failed: {status}");
    assert_eq!(mode_of(&directory), 0o700, "{fix} left the mode");
}

#[test]
fn each_single_quote_is_closed_escaped_and_reopened() {
    let error = PrivateSocketError::NotPrivate {
        path: at("/a/it's/b'c"),
        mode: 0o750,
    };
    assert_eq!(
        field(&from_socket(&error), "fix"),
        Some(&text(r#"chmod 700 '/a/it'"'"'s/b'"'"'c'"#))
    );
}

#[test]
fn bad_path_depends_on_its_origin() {
    let error = PrivateSocketError::BadPath {
        path: at("/Users/o/./a/control.uds"),
    };
    let flag = from_socket(&error);
    assert_eq!(flag.code(), ClientCode::InvalidArgument);
    assert_eq!(flag.code().exit(), Exit::Usage);
    assert_eq!(flag.fields(), fields(vec![("argument", text("--socket"))]));
    let home = from_home(&error);
    assert_eq!(home.code(), ClientCode::InvalidHome);
    assert_eq!(home.code().exit(), Exit::Refused);
    assert_eq!(home.fields(), &[]);
}

#[test]
fn peer_fields() {
    let mismatch = from_socket(&PrivateSocketError::PeerMismatch {
        trusted: 501,
        found: 65,
    });
    assert_eq!(mismatch.code(), ClientCode::PeerMismatch);
    assert_eq!(field(&mismatch, "uid"), Some(&number(501)));
    assert_eq!(field(&mismatch, "peer_uid"), Some(&number(65)));
    let unread = from_socket(&PrivateSocketError::PeerCredentials { errno: 57 });
    assert_eq!(unread.code(), ClientCode::PeerMismatch);
    assert_eq!(field(&unread, "uid"), Some(&number(501)));
    assert_eq!(field(&unread, "peer_uid"), None);
}

#[test]
fn the_trusted_uid_is_the_one_passed_in() {
    let effective = effective_uid();
    let passed = effective.wrapping_add(1);
    let own = effective.wrapping_add(2);
    assert_ne!(passed, effective);
    assert_ne!(own, effective);
    for error in [
        PrivateSocketError::PeerCredentials { errno: 57 },
        PrivateSocketError::PeerMismatch {
            trusted: own,
            found: 65,
        },
    ] {
        let refusal = map_refusal(&error, Path::new(SOCKET), PathOrigin::SocketFlag, passed);
        assert_eq!(
            field(&refusal, "uid"),
            Some(&number(passed)),
            "{}'s uid",
            variant_name(&error)
        );
    }
}

#[test]
fn a_bad_name_names_the_path_passed_to_the_check() {
    let error = PrivateSocketError::BadName {
        name: "a/b".to_string(),
    };
    let refusal = map_refusal(&error, Path::new(INSTANCES), PathOrigin::Home, TRUSTED);
    assert_eq!(
        refusal.fields(),
        fields(vec![("path", text(INSTANCES)), ("rule", text("name"))])
    );
}
