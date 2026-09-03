use plasmosome_work_state::project::{ProjectConfig, compiled_project_config};

const EXACT_CONFIG: &str = r#"
schema_version = 1
project_id = "plasmosome"
remote_name = "origin"
git_observation_url = "https://github.com/teonimesic/plasmosome.git"
dolt_remote_url = "git+https://github.com/teonimesic/plasmosome.git"
data_ref = "refs/dolt/data"
"#;

#[test]
fn project_config_accepts_only_the_compiled_plasmosome_remote_pair() {
    let compiled = compiled_project_config().expect("compiled project config is valid");
    assert_eq!(compiled.project_id(), "plasmosome");
    assert_eq!(compiled.remote_name(), "origin");
    assert_eq!(
        compiled.git_observation_url(),
        "https://github.com/teonimesic/plasmosome.git"
    );
    assert_eq!(
        compiled.dolt_remote_url(),
        "git+https://github.com/teonimesic/plasmosome.git"
    );
    assert_eq!(compiled.data_ref(), "refs/dolt/data");

    assert_eq!(ProjectConfig::parse(EXACT_CONFIG).unwrap(), compiled);
    for invalid in [
        EXACT_CONFIG.replace("schema_version = 1", "schema_version = 2"),
        EXACT_CONFIG.replace("project_id = \"plasmosome\"", "project_id = \"other\""),
        EXACT_CONFIG.replace("remote_name = \"origin\"", "remote_name = \"upstream\""),
        EXACT_CONFIG.replace(
            "https://github.com/teonimesic/plasmosome.git",
            "git+https://github.com/teonimesic/plasmosome.git",
        ),
        EXACT_CONFIG.replace(
            "git+https://github.com/teonimesic/plasmosome.git",
            "https://github.com/teonimesic/plasmosome.git",
        ),
        EXACT_CONFIG.replace("refs/dolt/data", "refs/heads/main"),
        format!("{EXACT_CONFIG}extra = \"value\"\n"),
        format!("{EXACT_CONFIG}remote_name = \"origin\"\n"),
    ] {
        assert_eq!(
            ProjectConfig::parse(&invalid)
                .expect_err("alternate project binding refuses")
                .code(),
            "invalid_project_config"
        );
    }
}
