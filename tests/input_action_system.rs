//! End to end tests for the input action schema.
//!
//! The fixtures live apart from the ones in `schema.rs` on purpose. Every file
//! under `invalid/` is a perfectly good `.model.json` that the model schema
//! accepts; what makes it invalid is the narrower shape this schema asks for.

use std::{fs, path::PathBuf};

use jsonschema::Validator;
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn schema() -> Value {
    let artifacts = rojo_schema::generate(&root()).expect("the vendored sources compile");
    let text = artifacts
        .get(rojo_schema::INPUT_ACTION_SYSTEM)
        .expect("the input action schema was compiled");

    serde_json::from_str(text).unwrap()
}

fn validator() -> Validator {
    jsonschema::validator_for(&schema()).expect("the compiled schema is a valid JSON Schema")
}

fn fixtures(kind: &str) -> Vec<(String, Value)> {
    let directory = root().join("tests/fixtures/input-action-system").join(kind);
    let mut files = Vec::new();

    for entry in fs::read_dir(&directory).expect("fixtures directory exists") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let document: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{name} is not valid JSON: {error}"));
        files.push((name, document));
    }

    assert!(
        !files.is_empty(),
        "no fixtures found in {}",
        directory.display()
    );
    files
}

#[test]
fn accepts_a_well_formed_tree() {
    let validator = validator();

    for (name, document) in fixtures("valid") {
        let errors: Vec<String> = validator
            .iter_errors(&document)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect();

        assert!(
            errors.is_empty(),
            "{name} should pass but did not:\n  {}",
            errors.join("\n  ")
        );
    }
}

#[test]
fn rejects_a_tree_that_breaks_the_shape() {
    let validator = validator();

    for (name, document) in fixtures("invalid") {
        assert!(
            !validator.is_valid(&document),
            "{name} should be rejected but passed"
        );
    }
}

/// The fixtures under `invalid/` have to be valid Rojo, or they would prove
/// nothing about this schema being the narrower one.
#[test]
fn every_rejected_file_is_still_a_model_rojo_reads() {
    let artifacts = rojo_schema::generate(&root()).expect("the vendored sources compile");
    let model: Value = serde_json::from_str(artifacts.get(rojo_schema::MODEL).unwrap()).unwrap();
    let model = jsonschema::validator_for(&model).unwrap();

    for (name, document) in fixtures("invalid") {
        // Rojo needs a class name, so the one fixture that drops it is the
        // exception: it is malformed for both schemas.
        if name == "no-class.model.json" {
            assert!(!model.is_valid(&document));
            continue;
        }

        assert!(
            model.is_valid(&document),
            "{name} is not a valid model file, so it does not test this schema"
        );
    }
}

#[test]
fn points_an_editor_at_the_published_url() {
    let schema = schema();

    assert_eq!(
        schema["$id"],
        "https://raw.githubusercontent.com/rbx-dev-tools/rojo-schema/main/schema/input-action-system.schema.json"
    );
    assert!(
        schema["$comment"]
            .as_str()
            .unwrap()
            .contains("reflection database"),
        "the schema should record which database it came from"
    );
}
