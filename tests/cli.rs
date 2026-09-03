use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use tempfile::{TempDir, tempdir};

fn project(name: &str) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("temporary project");
    let root = directory.path().to_path_buf();
    fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .expect("manifest");
    fs::write(
        root.join("Cargo.lock"),
        format!("version = 3\n\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("lockfile");
    (directory, root)
}

fn home(root: &Path) -> PathBuf {
    let cargo_home = root.join("cargo-home");
    fs::create_dir_all(&cargo_home).expect("Cargo home");
    cargo_home
}

fn export(root: &Path, cargo_home: &Path, extra: &[&str]) -> (String, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_envdrift"));
    command
        .current_dir(root)
        .env("CARGO_HOME", cargo_home)
        .args(["export", "--format", "json"])
        .args(extra);
    let output = command.output().expect("run export");
    assert!(
        output.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("UTF-8 JSON");
    let value = serde_json::from_str(&text).expect("valid report JSON");
    (text, value)
}

fn observations(value: &Value) -> Vec<String> {
    value["observations"]
        .as_array()
        .expect("observations array")
        .iter()
        .map(|observation| {
            format!(
                "{}:{}:{}:{}",
                observation["subject"].as_str().unwrap_or(""),
                observation["state"].as_str().unwrap_or(""),
                observation["value"].as_str().unwrap_or(""),
                observation["detail"].as_str().unwrap_or("")
            )
        })
        .collect()
}

#[test]
fn clean_project_is_stable_and_offline() {
    let (_directory, root) = project("clean-project");
    let cargo_home = home(&root);
    let (first_text, first) = export(&root, &cargo_home, &[]);
    let (second_text, second) = export(&root, &cargo_home, &[]);
    assert_eq!(first_text, second_text);
    assert_eq!(first, second);
    assert_eq!(first["network_access_attempted"], false);
    let state_text = observations(&first).join("\n");
    for state in ["declared", "locked", "installed", "observed", "built"] {
        assert!(
            state_text.contains(&format!(":{state}:")),
            "missing state {state}"
        );
    }
}

#[test]
fn extra_installed_binary_and_missing_cache_are_explicit() {
    let (_directory, root) = project("cache-project");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"cache-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nserde = \"1\"\n",
    )
    .expect("dependency manifest");
    fs::write(
        root.join("Cargo.lock"),
        "version = 3\n\n[[package]]\nname = \"cache-project\"\nversion = \"0.1.0\"\ndependencies = [\"serde\"]\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n",
    )
    .expect("dependency lockfile");
    let cargo_home = home(&root);
    fs::create_dir_all(cargo_home.join("bin")).expect("bin directory");
    fs::write(cargo_home.join("bin/extra-tool"), b"fixture").expect("installed binary");
    let (_text, value) = export(&root, &cargo_home, &[]);
    let state_text = observations(&value).join("\n");
    assert!(state_text.contains("installed:cargo-bin:extra-tool:installed:present"));
    assert!(state_text.contains("package:serde:downloaded:missing"));
    assert!(
        value["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap_or("")
                .contains("download state is missing"))
    );
}

#[test]
fn registry_source_and_toolchain_changes_are_explained() {
    let (_directory, root) = project("change-project");
    fs::create_dir_all(root.join(".cargo")).expect("config directory");
    fs::write(
        root.join(".cargo/config.toml"),
        "[source.crates-io]\nreplace-with = \"mirror-a\"\n[source.mirror-a]\nregistry = \"https://example.invalid/a\"\n",
    )
    .expect("source config");
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\n",
    )
    .expect("toolchain");
    let cargo_home = home(&root);
    let baseline = root.join("baseline.json");
    let output = Command::new(env!("CARGO_BIN_EXE_envdrift"))
        .current_dir(&root)
        .env("CARGO_HOME", &cargo_home)
        .args([
            "export",
            "--format",
            "json",
            "--output",
            baseline.to_str().expect("baseline path"),
        ])
        .output()
        .expect("write baseline");
    assert!(output.status.success());
    fs::write(
        root.join(".cargo/config.toml"),
        "[source.crates-io]\nreplace-with = \"mirror-b\"\n[source.mirror-b]\nregistry = \"https://example.invalid/b\"\n",
    )
    .expect("changed source config");
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"nightly\"\n",
    )
    .expect("changed toolchain");
    let explain = Command::new(env!("CARGO_BIN_EXE_envdrift"))
        .current_dir(&root)
        .env("CARGO_HOME", &cargo_home)
        .args([
            "explain",
            "--baseline",
            baseline.to_str().expect("baseline path"),
        ])
        .output()
        .expect("run explain");
    assert!(explain.status.success());
    let text = String::from_utf8(explain.stdout).expect("UTF-8 explanation");
    assert!(text.contains("mirror-b"));
    assert!(text.contains("channel=nightly"));
}

#[test]
fn malformed_lock_and_config_and_absolute_path_are_unresolved() {
    let (_directory, root) = project("malformed-project");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"malformed-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nlocal = { path = \"/opt/private/local\" }\n",
    )
    .expect("absolute path manifest");
    fs::write(root.join("Cargo.lock"), "[").expect("malformed lockfile");
    fs::create_dir_all(root.join(".cargo")).expect("config directory");
    fs::write(root.join(".cargo/config.toml"), "[").expect("malformed config");
    let cargo_home = home(&root);
    let (_text, value) = export(&root, &cargo_home, &[]);
    let warnings = value["warnings"].as_array().expect("warnings");
    let warning_text = warnings
        .iter()
        .map(|warning| warning.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(warning_text.contains("absolute local path dependency"));
    assert!(warning_text.contains("malformed Cargo.lock"));
    assert!(warning_text.contains("malformed Cargo config"));
}

#[test]
fn changed_toolchain_and_stale_target_are_reported() {
    let (_directory, root) = project("stale-project");
    let target = root.join("target/debug/deps");
    fs::create_dir_all(&target).expect("target metadata");
    fs::write(target.join("stale-project-old.d"), b"fixture").expect("target file");
    thread::sleep(Duration::from_millis(40));
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"stale-project\"\nversion = \"0.2.0\"\nedition = \"2021\"\n",
    )
    .expect("changed manifest");
    let cargo_home = home(&root);
    let (_text, value) = export(&root, &cargo_home, &[]);
    let state_text = observations(&value).join("\n");
    assert!(state_text.contains("target:staleness:observed:stale"));
    assert!(state_text.contains("target:staleness:unresolved"));
}

#[test]
fn workspace_package_selection_is_observed() {
    let directory = tempdir().expect("workspace");
    let root = directory.path();
    fs::create_dir_all(root.join("one/src")).expect("member one");
    fs::create_dir_all(root.join("two/src")).expect("member two");
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"one\", \"two\"]\nresolver = \"2\"\n",
    )
    .expect("workspace manifest");
    for name in ["one", "two"] {
        fs::write(
            root.join(name).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        )
        .expect("member manifest");
        fs::write(root.join(name).join("src/main.rs"), "fn main() {}\n").expect("member source");
    }
    fs::write(
        root.join("Cargo.lock"),
        "version = 3\n\n[[package]]\nname = \"one\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"two\"\nversion = \"0.1.0\"\n",
    )
    .expect("workspace lockfile");
    let cargo_home = home(root);
    let (_text, value) = export(root, &cargo_home, &["--manifest-path", "one/Cargo.toml"]);
    let state_text = observations(&value).join("\n");
    assert!(state_text.contains("workspace:packages:observed:one@0.1.0,two@0.1.0"));
}

#[test]
fn explain_without_baseline_is_an_explicit_unresolved_state() {
    let (_directory, root) = project("baseline-project");
    let cargo_home = home(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_envdrift"))
        .current_dir(&root)
        .env("CARGO_HOME", &cargo_home)
        .args(["explain", "--baseline", "missing.json"])
        .output()
        .expect("run explain");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("UTF-8 explanation");
    assert!(text.contains("baseline: missing"));
    assert!(text.contains("baseline file is missing"));
}
