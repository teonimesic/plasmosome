use plasmosome_membrane::runtime::path::{PathError, RecipePath};
use plasmosome_membrane::runtime::recipe::{PlatformLimits, RecipeRefusal, RuntimeRecipe};

#[test]
fn the_recipe_and_its_paths_are_cores_types_reached_through_membrane() {
    let path: plasmosome_core::runtime::path::RecipePath =
        RecipePath::parse("/var/cell/c1/root.raw").expect("a canonical path parses");
    assert_eq!(path.file_name(), "root.raw");
    assert_eq!(RecipePath::parse("var/cell"), Err(PathError::NotAbsolute));
    let limits: plasmosome_core::runtime::recipe::PlatformLimits =
        PlatformLimits::host().expect("this host reports its limits");
    let read: Result<plasmosome_core::runtime::recipe::RuntimeRecipe, RecipeRefusal> =
        RuntimeRecipe::parse(b"[]\n", &limits);
    let field = String::new();
    assert_eq!(read, Err(RecipeRefusal::NotAnObject { field }));
}

#[test]
fn a_config_reader_takes_its_cap_from_cores_record_bound() {
    use plasmosome_core::runtime::supervisor_config::{
        ConfigRecordRefusal, MAX_RECORD_BYTES, SupervisorConfig,
    };
    assert_eq!(MAX_RECORD_BYTES, 1_048_576);
    let limits = PlatformLimits::host().expect("this host reports its limits");
    let cap = MAX_RECORD_BYTES + 2;
    let read = vec![b'x'; cap];
    assert_eq!(
        SupervisorConfig::decode(&read, &limits),
        Err(ConfigRecordRefusal::TooLarge { bytes: cap })
    );
}
