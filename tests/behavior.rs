use std::{fs, process::Command};

#[test]
fn raw_execution_internals_are_not_in_the_shipped_public_api() {
    let project = tempfile::tempdir().unwrap();
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(
        project.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"cozydot-public-api-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ncozydot = {{ path = {manifest_dir:?} }}\n"
        ),
    )
    .unwrap();
    fs::write(
        project.path().join("src/main.rs"),
        r#"use cozydot::{operations, planner::lower_neutral, runner};

fn main() {
    let operation = operations::Operation::AptMetadataRefresh;
    let step = runner::Step::workflow(operation.clone());
    let _ = operations::execute_with_docker_lock_for_test(
        &operation,
        &[],
        std::path::Path::new("/tmp/lock"),
    );
    let _ = (step, lower_neutral::lower);
}
"#,
    )
    .unwrap();
    fs::copy(
        std::path::Path::new(manifest_dir).join("Cargo.lock"),
        project.path().join("Cargo.lock"),
    )
    .unwrap();

    let output = Command::new("cargo")
        .args(["check", "--quiet", "--offline"])
        .current_dir(project.path())
        .env("CARGO_TARGET_DIR", project.path().join("target"))
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "raw execution probe unexpectedly compiled"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for module in ["operations", "lower_neutral", "runner"] {
        assert!(
            stderr.contains(&format!("module `{module}` is private")),
            "missing privacy failure for {module}:\n{stderr}"
        );
    }
}
