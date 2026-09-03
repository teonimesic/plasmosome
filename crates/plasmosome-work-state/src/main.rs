use std::path::PathBuf;

use plasmosome_work_state::{
    contract::{contract_refusal_exit_code, parse_contract_request},
    read::{ReadCommand, project_read, render_human},
    run_contract,
    store::{
        BootstrapRequest, bootstrap, compiled_pin_manifest, generation_for_installed_wrapper,
        host_target, locate_store, locator_environment, read_disposable_snapshot,
        render_bootstrap_human,
    },
    sync::{SyncError, render_sync_human, synchronize},
};

enum Invocation {
    Bootstrap {
        source_ref: String,
        archive: PathBuf,
        binary: PathBuf,
        json: bool,
    },
    Read {
        command: ReadCommand,
        json: bool,
    },
    Sync {
        json: bool,
    },
}

fn main() {
    let values = std::env::args().skip(1).collect::<Vec<_>>();
    if values.first().map(String::as_str) == Some("contract-test") {
        run_contract_invocation(values);
        return;
    }
    let json = values.iter().any(|value| value == "--json");
    let invocation = parse_invocation(&values).unwrap_or_else(|code| fail(&code, json, 2));
    match invocation {
        Invocation::Bootstrap {
            source_ref,
            archive,
            binary,
            json,
        } => run_bootstrap(source_ref, archive, binary, json),
        Invocation::Read { command, json } => run_read(command, json),
        Invocation::Sync { json } => run_sync(json),
    }
}

fn run_contract_invocation(values: Vec<String>) {
    let request = parse_contract_request(values).unwrap_or_else(|code| {
        eprintln!("{code}");
        std::process::exit(2)
    });
    match run_contract(&request) {
        Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
        Err(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            eprintln!("{}", result.code);
            std::process::exit(contract_refusal_exit_code(&result.code));
        }
    }
}

fn parse_invocation(values: &[String]) -> Result<Invocation, String> {
    let command = values
        .first()
        .map(String::as_str)
        .ok_or_else(|| "invalid_command".to_owned())?;
    match command {
        "bootstrap" => parse_bootstrap(values),
        "list" => parse_read(values, ReadCommand::List),
        "ready" => parse_read(values, ReadCommand::Ready),
        "blocked" => parse_read(values, ReadCommand::Blocked),
        "show" => parse_show(values),
        "sync" => parse_sync(values),
        _ => Err("invalid_command".into()),
    }
}

fn parse_sync(values: &[String]) -> Result<Invocation, String> {
    match values {
        [command] if command == "sync" => Ok(Invocation::Sync { json: false }),
        [command, json] if command == "sync" && json == "--json" => {
            Ok(Invocation::Sync { json: true })
        }
        _ => Err("invalid_command".into()),
    }
}

fn parse_bootstrap(values: &[String]) -> Result<Invocation, String> {
    let mut source_ref = None;
    let mut archive = None;
    let mut binary = None;
    let mut json = false;
    let mut index = 1;
    while index < values.len() {
        match values[index].as_str() {
            "--json" if !json => {
                json = true;
                index += 1;
            }
            "--source-ref" => {
                let value = values
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "invalid_command".to_owned())?;
                if source_ref.is_some() {
                    return Err("invalid_command".into());
                }
                if value.trim().is_empty() || value.contains(['\n', '\r']) {
                    return Err("invalid_source_ref".into());
                }
                source_ref = Some(value.clone());
                index += 2;
            }
            "--archive" | "--bd" => {
                let value = values
                    .get(index + 1)
                    .filter(|value| !value.trim().is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| "invalid_command".to_owned())?;
                match values[index].as_str() {
                    "--archive" if archive.is_none() => archive = Some(PathBuf::from(value)),
                    "--bd" if binary.is_none() => binary = Some(PathBuf::from(value)),
                    _ => return Err("invalid_command".into()),
                }
                index += 2;
            }
            _ => return Err("invalid_command".into()),
        }
    }
    let source_ref = source_ref.ok_or_else(|| "invalid_command".to_owned())?;
    Ok(Invocation::Bootstrap {
        source_ref,
        archive: archive.ok_or_else(|| "invalid_command".to_owned())?,
        binary: binary.ok_or_else(|| "invalid_command".to_owned())?,
        json,
    })
}

fn parse_read(values: &[String], command: ReadCommand) -> Result<Invocation, String> {
    if values.len() == 1 {
        return Ok(Invocation::Read {
            command,
            json: false,
        });
    }
    if values.len() == 2 && values[1] == "--json" {
        return Ok(Invocation::Read {
            command,
            json: true,
        });
    }
    Err("invalid_command".into())
}

fn parse_show(values: &[String]) -> Result<Invocation, String> {
    let mut json = false;
    let mut key = None;
    for value in &values[1..] {
        if value == "--json" && !json {
            json = true;
        } else if !value.starts_with("--") && key.is_none() {
            key = Some(value.clone());
        } else {
            return Err("invalid_command".into());
        }
    }
    let key = key.ok_or_else(|| "invalid_command".to_owned())?;
    let valid = key.split_once(':').is_some_and(|(kind, id)| {
        matches!(kind, "intent" | "spec" | "task")
            && id.len() == 3
            && id.bytes().all(|byte| byte.is_ascii_digit())
            && key.matches(':').count() == 1
    });
    if !valid {
        return Err("invalid_document_key".into());
    }
    Ok(Invocation::Read {
        command: ReadCommand::Show(key),
        json,
    })
}

fn run_bootstrap(source_ref: String, archive: PathBuf, binary: PathBuf, json: bool) {
    let checkout =
        std::env::current_dir().unwrap_or_else(|_| fail("invalid_store_location", json, 1));
    let wrapper = std::env::current_exe().unwrap_or_else(|_| fail("invalid_store", json, 1));
    match bootstrap(&BootstrapRequest {
        checkout: checkout.clone(),
        source_root: checkout,
        source_ref,
        archive,
        binary,
        wrapper,
        host_target: host_target().into(),
    }) {
        Ok(result) if json => println!("{}", serde_json::to_string(&result).unwrap()),
        Ok(result) => print!("{}", render_bootstrap_human(&result)),
        Err(error) => fail(error.code(), json, bootstrap_exit_code(error.code())),
    }
}

fn run_read(command: ReadCommand, json: bool) {
    let checkout =
        std::env::current_dir().unwrap_or_else(|_| fail("invalid_store_location", json, 1));
    let environment = locator_environment().unwrap_or_else(|error| fail(error.code(), json, 1));
    let mut runner = plasmosome_work_state::command::SystemCommandRunner;
    let location = locate_store(&mut runner, &checkout, environment)
        .unwrap_or_else(|error| fail(error.code(), json, 1));
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| fail("invalid_store", json, 1));
    let generation = generation_for_installed_wrapper(&location, &executable)
        .unwrap_or_else(|error| fail(error.code(), json, 1));
    let pin = compiled_pin_manifest().unwrap_or_else(|error| fail(error.code(), json, 1));
    let snapshot = read_disposable_snapshot(&mut runner, &generation, &pin, host_target())
        .unwrap_or_else(|error| fail(error.code(), json, 1));
    let response = project_read(
        command,
        &snapshot,
        &generation.manifest.authority_mode,
        &generation.manifest.source_commit,
    )
    .unwrap_or_else(|error| {
        let document_key = (error.code() == "document_not_found")
            .then_some(error.document_key.as_deref())
            .flatten();
        fail_with_document_key(
            error.code(),
            json,
            read_exit_code(error.code()),
            document_key,
        )
    });
    if json {
        println!("{}", serde_json::to_string(&response).unwrap());
    } else {
        print!("{}", render_human(&response));
    }
}

fn run_sync(json: bool) {
    let checkout =
        std::env::current_dir().unwrap_or_else(|_| fail_sync_code("invalid_store_location", json));
    let environment =
        locator_environment().unwrap_or_else(|error| fail_sync_code(error.code(), json));
    let mut runner = plasmosome_work_state::command::SystemCommandRunner;
    let location = locate_store(&mut runner, &checkout, environment)
        .unwrap_or_else(|error| fail_sync_code(error.code(), json));
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| fail_sync_code("invalid_store", json));
    let generation = generation_for_installed_wrapper(&location, &executable)
        .unwrap_or_else(|error| fail_sync_code(error.code(), json));
    let pin = compiled_pin_manifest().unwrap_or_else(|error| fail_sync_code(error.code(), json));
    match synchronize(&mut runner, &location, &generation, &pin, host_target()) {
        Ok(result) if json => println!("{}", serde_json::to_string(&result).unwrap()),
        Ok(result) => print!("{}", render_sync_human(&result)),
        Err(error) => fail_sync(error, json),
    }
}

fn bootstrap_exit_code(code: &str) -> i32 {
    if code == "invalid_source_ref" { 2 } else { 1 }
}

fn read_exit_code(code: &str) -> i32 {
    if code == "invalid_document_key" { 2 } else { 1 }
}

fn fail(code: &str, json: bool, exit_code: i32) -> ! {
    fail_with_document_key(code, json, exit_code, None)
}

fn fail_sync(error: SyncError, json: bool) -> ! {
    fail_sync_refusal(error.code(), error.state_changed(), json)
}

fn fail_sync_code(code: &str, json: bool) -> ! {
    fail_sync_refusal(code, false, json)
}

fn fail_sync_refusal(code: &str, state_changed: bool, json: bool) -> ! {
    eprint!("{}", render_sync_refusal(code, state_changed, json));
    std::process::exit(1)
}

fn render_sync_refusal(code: &str, state_changed: bool, json: bool) -> String {
    if json {
        format!(
            "{}\n",
            serde_json::to_string(&serde_json::json!({
                "code": code,
                "state_changed": state_changed,
            }))
            .unwrap()
        )
    } else {
        format!("error[{code}]: {code} state_changed={state_changed}\n")
    }
}

fn fail_with_document_key(code: &str, json: bool, exit_code: i32, document_key: Option<&str>) -> ! {
    if json {
        if let Some(document_key) = document_key {
            eprintln!(
                "{}",
                serde_json::json!({ "code": code, "document_key": document_key })
            );
        } else {
            eprintln!("{}", serde_json::json!({ "code": code }));
        }
    } else if let Some(document_key) = document_key {
        eprintln!("error[{code}]: {code} ({document_key})");
    } else {
        eprintln!("error[{code}]: {code}");
    }
    std::process::exit(exit_code)
}

#[cfg(test)]
mod tests {
    use super::render_sync_refusal;

    #[test]
    fn sync_runtime_refusals_render_state_changed_without_leaks() {
        let human = render_sync_refusal("remote_transport", true, false);
        let json = render_sync_refusal("remote_transport", true, true);

        assert_eq!(
            human,
            "error[remote_transport]: remote_transport state_changed=true\n"
        );
        assert_eq!(
            json,
            "{\"code\":\"remote_transport\",\"state_changed\":true}\n"
        );
        for leaked in [
            "https://",
            "git+https://",
            "/Users/",
            "/tmp/",
            "HOME=",
            "PATH=",
            "credential",
            "archive",
        ] {
            assert!(!human.contains(leaked));
            assert!(!json.contains(leaked));
        }
    }
}
