#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{ConfigError, DaemonConfig, read_config};
    use crate::runtime::cell_id::{MAX_CELL_NUMBER, format_cell_id};
    use crate::runtime::fixtures::{LIMITS, artifact, digest, valid_recipe};
    use crate::runtime::path::{PathError, RecipePath};
    use crate::runtime::recipe::{
        Artifact, ArtifactRole, PathRole, RecipeRefusal, RuntimeRecipe, RuntimeTemplate,
        TemplateRefusal,
    };
    use crate::runtime::supervisor_config::{
        ConfigRecordRefusal, MAX_RECORD_BYTES, SupervisorConfig,
    };
    use serde_json::{Map, Value, json};
    use std::error::Error as _;
    use std::time::Duration;

    const ROOT: &str = "/srv/roots/r7";
    const LONGEST: &str = "cell-9999999999";
    const MEMBRANED: &str = "/opt/plasmosome/membraned";
    const TEMPLATE_KEYS: [&str; 12] = [
        "version",
        "vcpus",
        "memory_mib",
        "kernel",
        "kernel_format",
        "initramfs",
        "root_image",
        "helper",
        "libraries",
        "host_policy",
        "guest_policy",
        "architecture",
    ];

    fn template() -> Value {
        let mut template = valid_recipe();
        let fields = template
            .as_object_mut()
            .expect("the fixture recipe is an object");
        for key in ["writable_root", "control_path", "data_path"] {
            fields.remove(key);
        }
        template
    }

    fn runtime() -> Value {
        json!({
            "membraned": artifact(MEMBRANED, '8'),
            "template": template(),
            "start_deadline_ms": 60000,
            "stop_deadline_ms": 45000,
        })
    }

    fn config_text(root: &str, cell_runtime: Option<&str>) -> String {
        let mut text = format!(
            r#"{{"control_socket": "/tmp/c.uds", "name": "work", "instance_root": {}, "recovery_deadline_ms": 4321"#,
            Value::from(root)
        );
        if let Some(cell_runtime) = cell_runtime {
            text.push_str(&format!(r#", "cell_runtime": {cell_runtime}"#));
        }
        text.push('}');
        text
    }

    fn read(text: &str) -> Result<DaemonConfig, ConfigError> {
        read_config(text, || Ok(LIMITS))
    }

    fn read_under(root: &str, cell_runtime: &Value) -> Result<DaemonConfig, ConfigError> {
        read(&config_text(root, Some(&cell_runtime.to_string())))
    }

    fn accepted(cell_runtime: &Value) -> CellRuntime {
        match read_under(ROOT, cell_runtime) {
            Ok(DaemonConfig {
                cell_runtime: Some(parsed),
                ..
            }) => parsed,
            other => panic!("{cell_runtime} is a valid cell_runtime, got {other:?}"),
        }
    }

    fn refused_under(root: &str, cell_runtime: &Value) -> CellRuntimeError {
        match read_under(root, cell_runtime) {
            Err(ConfigError::CellRuntime(refusal)) => refusal,
            other => panic!("{cell_runtime} under {root} is a cell_runtime refusal, got {other:?}"),
        }
    }

    fn refused(cell_runtime: &Value) -> CellRuntimeError {
        refused_under(ROOT, cell_runtime)
    }

    fn assert_names(refusal: &CellRuntimeError, name: &str) {
        let shown = ConfigError::CellRuntime(refusal.clone()).to_string();
        assert!(shown.contains(name), "the refusal names {name}: {shown}");
    }

    fn changed(edit: impl FnOnce(&mut Map<String, Value>)) -> Value {
        let mut value = runtime();
        edit(value.as_object_mut().expect("cell_runtime is an object"));
        value
    }

    fn with_template(edit: impl FnOnce(&mut Map<String, Value>)) -> Value {
        changed(|fields| {
            edit(
                fields["template"]
                    .as_object_mut()
                    .expect("the template is an object"),
            )
        })
    }

    fn per_cell(root: &str, cell: &str, name: &str) -> RecipePath {
        RecipePath::parse(&format!("{root}/cells/{cell}/runtime/{name}"))
            .expect("the test builds a canonical path")
    }

    fn completed(template: &RuntimeTemplate, cell: &str) -> Result<RuntimeRecipe, RecipeRefusal> {
        template.complete(
            &per_cell(ROOT, cell, "root.img"),
            &per_cell(ROOT, cell, "control.uds"),
            &per_cell(ROOT, cell, "data.uds"),
            &LIMITS,
        )
    }

    fn root_of(bytes: usize) -> String {
        format!("/srv/{}", "r".repeat(bytes - "/srv/".len()))
    }

    #[test]
    fn a_cell_runtime_is_read_with_the_recipe_its_startup_check_completed_for_the_longest_id() {
        let template = RuntimeTemplate::from_value(&template()).expect("the fixture is a template");
        let startup_recipe =
            completed(&template, LONGEST).expect("the fixture completes for the longest ID");
        assert_eq!(
            accepted(&runtime()),
            CellRuntime {
                membraned: Artifact::from_value(&artifact(MEMBRANED, '8'), "membraned")
                    .expect("the fixture is an artifact"),
                template,
                startup_recipe,
                instance_root: RecipePath::parse(ROOT).expect("canonical"),
                start_deadline: Duration::from_millis(60_000),
                stop_deadline: Duration::from_millis(45_000),
            }
        );
    }

    #[test]
    fn without_cell_runtime_nothing_changes_and_the_host_is_never_asked_for_its_limits() {
        for root in [ROOT, "/srv/roots/r7/", "/srv//r7", "/srv/../r7", "/"] {
            let parsed = read_config(&config_text(root, None), || {
                panic!("the host's limits are read only for a cell_runtime")
            })
            .unwrap_or_else(|error| panic!("{root} is an absolute root: {error}"));
            assert_eq!(parsed.cell_runtime, None, "{root}");
        }
    }

    #[test]
    fn a_host_that_will_not_report_its_limits_refuses_a_cell_runtime() {
        let unavailable = RecipeRefusal::HostLimitUnavailable {
            limit: "physical memory",
        };
        let refusal = read_config(&config_text(ROOT, Some(&runtime().to_string())), || {
            Err(unavailable.clone())
        })
        .expect_err("a cell_runtime needs the host's limits");
        assert!(
            matches!(
                &refusal,
                ConfigError::CellRuntime(CellRuntimeError::Host(found)) if *found == unavailable
            ),
            "{refusal:?}"
        );
        assert!(refusal.to_string().contains("physical memory"), "{refusal}");
    }

    #[test]
    fn a_cell_runtime_that_is_not_an_object_is_refused_null_included() {
        for written in ["null", "7", r#""cell""#, "[]", "true"] {
            let refusal = read(&config_text(ROOT, Some(written)))
                .expect_err("cell_runtime must be an object");
            assert!(
                matches!(
                    refusal,
                    ConfigError::CellRuntime(CellRuntimeError::NotAnObject)
                ),
                "{written}: {refusal:?}"
            );
            assert!(refusal.to_string().contains("`cell_runtime`"), "{refusal}");
        }
    }

    #[test]
    fn cell_runtime_holds_its_three_required_keys_and_an_optional_stop_deadline_only() {
        let unknown = refused(&changed(|fields| {
            fields.insert("zz_extra".into(), json!(1));
            fields.insert("deadline_ms".into(), json!(1));
        }));
        assert_eq!(
            unknown,
            CellRuntimeError::UnknownKey {
                key: "deadline_ms".to_owned()
            }
        );
        assert_names(&unknown, "`cell_runtime.deadline_ms`");

        for key in ["membraned", "template", "start_deadline_ms"] {
            let missing = refused(&changed(|fields| {
                fields.remove(key);
            }));
            assert_eq!(missing, CellRuntimeError::MissingKey { key }, "{key}");
            assert_names(&missing, &format!("`cell_runtime.{key}`"));
        }
        assert_eq!(
            refused(&changed(|fields| {
                fields.remove("template");
                fields.remove("start_deadline_ms");
            })),
            CellRuntimeError::MissingKey { key: "template" },
            "the first missing key in key order is named"
        );
        assert_eq!(
            refused(&changed(|fields| {
                fields.remove("membraned");
                fields.insert("extra".into(), json!(1));
            })),
            CellRuntimeError::UnknownKey {
                key: "extra".to_owned()
            },
            "an unknown key is refused before a missing one"
        );
    }

    #[test]
    fn a_template_that_carries_a_per_cell_path_lacks_a_key_or_adds_one_is_refused_by_name() {
        for key in ["writable_root", "control_path", "data_path"] {
            let refusal = refused(&with_template(|fields| {
                fields.insert(key.into(), json!("/var/cell/c1/x"));
            }));
            assert_eq!(
                refusal,
                CellRuntimeError::Template(TemplateRefusal::PerCellField { field: key })
            );
            assert_names(&refusal, key);
        }
        for key in TEMPLATE_KEYS {
            let refusal = refused(&with_template(|fields| {
                fields.remove(key);
            }));
            assert_eq!(
                refusal,
                CellRuntimeError::Template(TemplateRefusal::Recipe(RecipeRefusal::MissingField {
                    field: key.to_owned()
                })),
                "{key}"
            );
            assert_names(&refusal, key);
        }
        let unknown = refused(&with_template(|fields| {
            fields.insert("sizing".into(), json!(1));
        }));
        assert_eq!(
            unknown,
            CellRuntimeError::Template(TemplateRefusal::Recipe(RecipeRefusal::UnknownField {
                field: "sizing".to_owned()
            }))
        );
        assert_names(&unknown, "`cell_runtime.template`");
        assert_eq!(
            refused(&changed(|fields| {
                fields.insert("template".into(), json!(7));
            })),
            CellRuntimeError::Template(TemplateRefusal::Recipe(RecipeRefusal::NotAnObject {
                field: String::new()
            }))
        );
    }

    #[test]
    fn a_template_with_a_value_a_recipe_refuses_refuses_startup() {
        for (key, value, expected) in [
            ("vcpus", json!(0), RecipeRefusal::ZeroVcpus),
            (
                "vcpus",
                json!(9),
                RecipeRefusal::VcpusAboveLimit {
                    requested: 9,
                    limit: 8,
                },
            ),
            ("memory_mib", json!(0), RecipeRefusal::ZeroMemory),
            (
                "architecture",
                json!("sparc"),
                RecipeRefusal::UnknownArchitecture {
                    found: "sparc".to_owned(),
                },
            ),
            (
                "kernel_format",
                json!("zip"),
                RecipeRefusal::UnknownKernelFormat {
                    found: "zip".to_owned(),
                },
            ),
            (
                "version",
                json!(2),
                RecipeRefusal::UnsupportedVersion { found: 2 },
            ),
            (
                "kernel",
                json!({"path": "opt/kernel", "sha256": digest('1')}),
                RecipeRefusal::BadPath {
                    field: "kernel.path".to_owned(),
                    fault: PathError::NotAbsolute,
                },
            ),
        ] {
            let refusal = refused(&with_template(|fields| {
                fields.insert(key.into(), value.clone());
            }));
            assert_eq!(
                refusal,
                CellRuntimeError::Template(TemplateRefusal::Recipe(expected)),
                "{key}: {value}"
            );
            assert_names(&refusal, "`cell_runtime.template`");
        }
    }

    #[test]
    fn membraned_is_an_artifact_named_from_cell_runtime() {
        for (value, expected) in [
            (
                json!(7),
                RecipeRefusal::NotAnObject {
                    field: "cell_runtime.membraned".to_owned(),
                },
            ),
            (
                json!({"path": MEMBRANED}),
                RecipeRefusal::MissingField {
                    field: "cell_runtime.membraned.sha256".to_owned(),
                },
            ),
            (
                json!({"path": "membraned", "sha256": digest('8')}),
                RecipeRefusal::BadPath {
                    field: "cell_runtime.membraned.path".to_owned(),
                    fault: PathError::NotAbsolute,
                },
            ),
        ] {
            let refusal = refused(&changed(|fields| {
                fields.insert("membraned".into(), value.clone());
            }));
            assert_eq!(refusal, CellRuntimeError::Membraned(expected), "{value}");
            assert_names(&refusal, "cell_runtime.membraned");
        }
    }

    #[test]
    fn a_key_repeated_anywhere_inside_cell_runtime_is_refused_even_when_the_last_copy_is_valid() {
        let text = runtime().to_string();
        for (once, twice, pointer) in [
            (
                r#""start_deadline_ms":60000"#,
                r#""start_deadline_ms":1,"start_deadline_ms":60000"#,
                "/start_deadline_ms",
            ),
            (r#""vcpus":2"#, r#""vcpus":4,"vcpus":2"#, "/template/vcpus"),
            (
                r#""membraned":{"path""#,
                r#""membraned":{"path":"/elsewhere","path""#,
                "/membraned/path",
            ),
            (
                r#"[{"path":"/opt/cell/lib/libkrun.1.dylib""#,
                r#"[{"path":"/x","path":"/opt/cell/lib/libkrun.1.dylib""#,
                "/template/libraries/0/path",
            ),
        ] {
            assert!(text.contains(once), "{once} is in {text}");
            let spliced = text.replacen(once, twice, 1);
            let refusal = read(&config_text(ROOT, Some(&spliced)))
                .expect_err("a repeated key inside cell_runtime is refused");
            assert!(
                matches!(refusal, ConfigError::NotConfig(_)),
                "{spliced}: {refusal:?}"
            );
            let shown = refusal.to_string();
            assert!(
                shown.contains("`cell_runtime`") && shown.contains(pointer),
                "the refusal names cell_runtime and {pointer}: {shown}"
            );
        }
    }

    #[test]
    fn deadlines_are_positive_whole_milliseconds_and_stop_defaults_to_thirty_seconds() {
        let defaulted = accepted(&changed(|fields| {
            fields.remove("stop_deadline_ms");
        }));
        assert_eq!(defaulted.stop_deadline, Duration::from_millis(30_000));
        assert_eq!(defaulted.start_deadline, Duration::from_millis(60_000));

        for key in ["start_deadline_ms", "stop_deadline_ms"] {
            for (written, millis) in [("1", 1), ("18446744073709551615", u64::MAX)] {
                let value: Value = serde_json::from_str(written).expect("a JSON number");
                let parsed = accepted(&changed(|fields| {
                    fields.insert(key.into(), value);
                }));
                let deadline = if key == "start_deadline_ms" {
                    parsed.start_deadline
                } else {
                    parsed.stop_deadline
                };
                assert_eq!(deadline, Duration::from_millis(millis), "{key}: {written}");
            }
            for written in [
                "0",
                "-1",
                "1.5",
                "1e3",
                "18446744073709551616",
                r#""5000""#,
                "null",
                "true",
            ] {
                let value: Value = serde_json::from_str(written).expect("a JSON value");
                let refusal = refused(&changed(|fields| {
                    fields.insert(key.into(), value);
                }));
                assert_eq!(
                    refusal,
                    CellRuntimeError::BadDeadline { key },
                    "{key}: {written}"
                );
                assert_names(&refusal, &format!("`cell_runtime.{key}`"));
            }
        }
    }

    #[test]
    fn with_cell_runtime_the_instance_root_must_be_a_path_a_recipe_accepts() {
        for (root, fault) in [
            ("/srv/roots/r7/", PathError::TrailingSlash),
            ("/srv//r7", PathError::EmptyComponent),
            ("/srv/./r7", PathError::DotComponent),
            ("/srv/../r7", PathError::DotDotComponent),
            ("/", PathError::Root),
        ] {
            let refusal = refused_under(root, &runtime());
            assert_eq!(refusal, CellRuntimeError::InstanceRoot { fault }, "{root}");
            assert_names(&refusal, "`instance_root`");
        }
    }

    #[test]
    fn a_root_too_long_for_the_longest_cells_paths_is_refused_before_its_sockets_are_measured() {
        let max = libc::PATH_MAX as usize - 1;
        let root = root_of(max - "/cells/cell-9999999999/runtime/control.uds".len() + 1);
        let refusal = refused_under(&root, &runtime());
        assert_eq!(
            refusal,
            CellRuntimeError::CellPaths(CellPathsError::Path(PathError::TooLong {
                bytes: max + 1,
                max,
            }))
        );
        assert_names(&refusal, "`instance_root`");
    }

    #[test]
    fn each_socket_path_of_the_longest_cell_id_must_fit_in_the_hosts_socket_bound() {
        assert_eq!(LIMITS.max_socket_path_bytes(), 103);
        for name in ["membrane.uds", "runtime/data.uds", "runtime/control.uds"] {
            let suffix = format!("/cells/{LONGEST}/{name}").len();
            let over = root_of(104 - suffix);
            let path = format!("{over}/cells/{LONGEST}/{name}");
            let refusal = refused_under(&over, &runtime());
            assert_eq!(
                refusal,
                CellRuntimeError::SocketPathTooLong {
                    path: path.clone(),
                    bytes: 104,
                    max: 103,
                },
                "{name}"
            );
            assert_names(&refusal, &path);

            let fits = root_of(103 - suffix);
            let fitting = format!("{fits}/cells/{LONGEST}/{name}");
            match read_under(&fits, &runtime()) {
                Ok(_) => {}
                Err(ConfigError::CellRuntime(CellRuntimeError::SocketPathTooLong {
                    path, ..
                })) => assert_ne!(path, fitting, "a {name} of 103 bytes fits"),
                Err(other) => panic!("{fits} gives only socket refusals: {other:?}"),
            }
        }
        let fits = root_of(103 - format!("/cells/{LONGEST}/runtime/control.uds").len());
        assert!(
            read_under(&fits, &runtime()).is_ok(),
            "a root whose longest socket path is 103 bytes is accepted"
        );
    }

    #[test]
    fn a_template_artifact_inside_the_cells_directory_is_refused_so_no_cell_can_reuse_its_path() {
        let collides = format!("{ROOT}/cells/cell-1/runtime/root.img");
        let value = with_template(|fields| {
            fields.insert(
                "root_image".into(),
                json!({"path": collides, "sha256": digest('3')}),
            );
        });
        let template = RuntimeTemplate::from_value(&value["template"]).expect("a template");
        assert!(
            completed(&template, LONGEST).is_ok(),
            "the longest ID alone cannot see the collision"
        );
        assert_eq!(
            completed(&template, "cell-1"),
            Err(RecipeRefusal::PathReused {
                first: PathRole::Artifact(ArtifactRole::RootImage),
                second: PathRole::WritableRoot,
            }),
            "cell-1's recipe would be refused after its directory exists"
        );
        let refusal = refused(&value);
        assert_eq!(
            refusal,
            CellRuntimeError::ArtifactInCells {
                role: ArtifactRole::RootImage,
                path: collides.clone(),
            }
        );
        assert_names(&refusal, &collides);
        assert_names(&refusal, "root_image");

        let library = format!("{ROOT}/cells/kept/libkrun.dylib");
        assert_eq!(
            refused(&with_template(|fields| {
                fields.insert("libraries".into(), json!([artifact(&library, '5')]));
            })),
            CellRuntimeError::ArtifactInCells {
                role: ArtifactRole::Library(0),
                path: library,
            }
        );

        for beside in [
            format!("{ROOT}/cells-old/kernel"),
            format!("{ROOT}/cellsx"),
            format!("{ROOT}/retired/cell-1/runtime/root.img"),
            format!("{ROOT}/kernel"),
        ] {
            accepted(&with_template(|fields| {
                fields.insert("kernel".into(), artifact(&beside, '1'));
            }));
        }
    }

    #[test]
    fn a_template_whose_largest_config_record_is_over_the_bound_refuses_startup() {
        let library = |index: usize, pad: usize| {
            artifact(
                &format!("/opt/cell/lib/{}lib{index:06}.dylib", "p".repeat(pad)),
                '5',
            )
        };
        let with_libraries = |count: usize, pad: usize| {
            let mut libraries: Vec<Value> = (0..count).map(|index| library(index, 0)).collect();
            libraries[0] = library(0, pad);
            with_template(|fields| {
                fields.insert("libraries".into(), Value::Array(libraries));
            })
        };
        let record_bytes = |value: &Value| {
            let template = RuntimeTemplate::from_value(&value["template"]).expect("a template");
            let record = SupervisorConfig {
                cell: format_cell_id(MAX_CELL_NUMBER),
                instance_root: RecipePath::parse(ROOT).expect("canonical"),
                runtime_recipe: completed(&template, LONGEST).expect("the template completes"),
                boot_deadline_ms: u64::MAX,
            }
            .encode()
            .expect("the record is under the bound");
            record.len() - 1
        };
        let one = record_bytes(&with_libraries(1, 0));
        let each = record_bytes(&with_libraries(2, 0)) - one;
        let count = 1 + (MAX_RECORD_BYTES - one) / each;
        let pad = MAX_RECORD_BYTES - record_bytes(&with_libraries(count, 0));
        let largest = with_libraries(count, pad);
        assert_eq!(record_bytes(&largest), MAX_RECORD_BYTES);
        accepted(&largest);

        let refusal = refused(&with_libraries(count, pad + 1));
        assert_eq!(
            refusal,
            CellRuntimeError::Record(ConfigRecordRefusal::TooLarge {
                bytes: MAX_RECORD_BYTES + 1
            })
        );
        assert_names(&refusal, "1048576");
    }

    #[test]
    fn the_artifacts_to_verify_are_membraned_then_every_template_artifact_in_recipe_order() {
        let parsed = accepted(&with_template(|fields| {
            fields.insert(
                "libraries".into(),
                json!([
                    artifact("/opt/cell/lib/a.dylib", '5'),
                    artifact("/opt/cell/lib/b.dylib", '9')
                ]),
            );
        }));
        let paths: Vec<&str> = parsed
            .artifacts()
            .map(|artifact| artifact.path().as_str())
            .collect();
        assert_eq!(
            paths,
            [
                MEMBRANED,
                "/opt/cell/kernel",
                "/opt/cell/initramfs",
                "/opt/cell/root.raw",
                "/opt/cell/krun-helper",
                "/opt/cell/lib/a.dylib",
                "/opt/cell/lib/b.dylib",
                "/opt/cell/host.sb",
                "/opt/cell/guest.json",
            ]
        );
    }

    #[test]
    fn a_cell_runtime_refusal_is_the_config_errors_cause() {
        let refusal = read(&config_text(ROOT, Some("7"))).expect_err("refused");
        let cause = refusal
            .source()
            .and_then(|source| source.downcast_ref::<CellRuntimeError>());
        assert_eq!(cause, Some(&CellRuntimeError::NotAnObject));
        assert_eq!(
            refusal.to_string(),
            format!(
                "`cell_runtime` is refused: {}",
                CellRuntimeError::NotAnObject
            )
        );
    }
}
