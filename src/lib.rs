use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

type Result<T> = std::result::Result<T, EnvdriftError>;

#[derive(Debug)]
pub struct EnvdriftError(String);

impl fmt::Display for EnvdriftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EnvdriftError {}

fn error(message: impl Into<String>) -> EnvdriftError {
    EnvdriftError(message.into())
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Json,
    Text,
}

#[derive(Debug, Parser)]
#[command(
    name = "envdrift",
    version,
    about = "Detect drift between Rust declarations and local state"
)]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Scan {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    Explain {
        #[arg(long, default_value = ".envdrift/baseline.json", value_name = "PATH")]
        baseline: PathBuf,
    },
    Export {
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
    Version,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Observation {
    pub subject: String,
    pub state: String,
    pub value: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub cargo: Option<String>,
    pub rustc: Option<String>,
    pub active_toolchain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Report {
    pub schema_version: u32,
    pub tool: String,
    pub platform: String,
    pub manifest_path: String,
    pub workspace_root: String,
    pub network_access_attempted: bool,
    pub runtime: RuntimeIdentity,
    pub observations: Vec<Observation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoMetadataPackage>,
    workspace_root: String,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataPackage {
    name: String,
    version: String,
    manifest_path: String,
}

#[derive(Debug, Clone)]
struct DeclaredDependency {
    specs: Vec<String>,
    sections: BTreeSet<String>,
    path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

#[derive(Debug, Clone)]
struct ConfigInfo {
    path: PathBuf,
    target_dir: Option<String>,
}

#[derive(Debug, Clone)]
struct ToolchainFile {
    path: PathBuf,
    digest: String,
}

pub fn entry<I, T>(args: I) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args).map_err(|e| error(e.to_string()))?;
    if let CliCommand::Version = cli.command {
        println!("envdrift {VERSION}");
        return Ok(0);
    }

    let manifest = locate_manifest(cli.manifest_path.as_deref())?;
    match cli.command {
        CliCommand::Scan { format, output } => {
            let report = scan_project_from_manifest(&manifest)?;
            emit(render_report(&report, format)?, output.as_deref())?;
        }
        CliCommand::Explain { baseline } => {
            let report = scan_project_from_manifest(&manifest)?;
            let baseline = resolve_from_current_dir(&baseline);
            let text = if baseline.is_file() {
                let baseline_text = fs::read_to_string(&baseline).map_err(|e| {
                    error(format!(
                        "could not read baseline {}: {e}",
                        baseline.display()
                    ))
                })?;
                let baseline_report: Report =
                    serde_json::from_str(&baseline_text).map_err(|e| {
                        error(format!(
                            "could not parse baseline {}: {e}",
                            baseline.display()
                        ))
                    })?;
                explain_reports(&baseline_report, &report)
            } else {
                explain_without_baseline(&report, &baseline)
            };
            print!("{text}");
        }
        CliCommand::Export { format, output } => {
            let report = scan_project_from_manifest(&manifest)?;
            emit(render_report(&report, format)?, output.as_deref())?;
        }
        CliCommand::Version => unreachable!(),
    }
    Ok(0)
}

pub fn scan_project(manifest_path: Option<&Path>) -> Result<Report> {
    let manifest = locate_manifest(manifest_path)?;
    scan_project_from_manifest(&manifest)
}

fn scan_project_from_manifest(manifest: &Path) -> Result<Report> {
    let manifest = canonical_or_original(manifest)?;
    let manifest_parent = manifest
        .parent()
        .ok_or_else(|| error("Cargo.toml has no parent directory"))?;
    let manifest_bytes = fs::read(&manifest)
        .map_err(|e| error(format!("could not read {}: {e}", manifest.display())))?;
    let manifest_value = match toml::from_str::<toml::Value>(
        std::str::from_utf8(&manifest_bytes)
            .map_err(|e| error(format!("Cargo.toml is not UTF-8: {e}")))?,
    ) {
        Ok(value) => Some(value),
        Err(parse_error) => {
            let mut report = empty_report(&manifest, manifest_parent);
            add_unresolved(
                &mut report,
                "manifest",
                format!(
                    "malformed Cargo.toml: {}",
                    first_line(&parse_error.to_string())
                ),
            );
            report
                .warnings
                .push("Cargo.toml could not be parsed".to_string());
            return Ok(report);
        }
    };

    let metadata = run_cargo_metadata(&manifest, manifest_parent);
    let workspace_root = metadata
        .as_ref()
        .map(|data| PathBuf::from(&data.workspace_root))
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| manifest_parent.to_path_buf());
    let mut report = empty_report(&manifest, &workspace_root);
    add_observation(
        &mut report,
        "scan",
        "observed",
        Some("offline".to_string()),
        "The scan invokes Cargo with --offline; no network access is attempted",
    );

    let (manifest_digest, manifest_complete) = digest_bytes(&manifest_bytes, 8 * 1024 * 1024);
    add_observation(
        &mut report,
        &format!("file:{}", display_path(&manifest, &workspace_root)),
        "declared",
        Some(digest_value(&manifest_digest, manifest_complete)),
        "Cargo.toml content digest",
    );

    let declarations = collect_declarations(
        manifest_value
            .as_ref()
            .expect("manifest value exists after the parse branch"),
    );
    let package_name = manifest_value
        .as_ref()
        .and_then(|value| value.get("package"))
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(ToString::to_string);
    if let Some(name) = package_name.as_ref() {
        let version = manifest_value
            .as_ref()
            .and_then(|value| value.get("package"))
            .and_then(|package| package.get("version"))
            .and_then(toml::Value::as_str)
            .unwrap_or("unresolved");
        add_observation(
            &mut report,
            &format!("package:{name}"),
            "declared",
            Some(version.to_string()),
            "root package in Cargo.toml",
        );
    } else {
        add_unresolved(
            &mut report,
            "manifest",
            "package.name is missing".to_string(),
        );
    }
    for (name, dependency) in &declarations {
        let value = dependency.specs.join("; ");
        add_observation(
            &mut report,
            &format!("package:{name}"),
            "declared",
            Some(value),
            &format!("dependency sections: {}", join_set(&dependency.sections)),
        );
        if let Some(path) = &dependency.path {
            let was_absolute = path.is_absolute();
            let resolved_path = if path.is_absolute() {
                path.clone()
            } else {
                manifest_parent.join(path)
            };
            if was_absolute {
                add_unresolved(
                    &mut report,
                    &format!("package:{name}"),
                    format!(
                        "absolute local path dependency: {}",
                        display_path(&resolved_path, &workspace_root)
                    ),
                );
            } else {
                add_observation(
                    &mut report,
                    &format!("package:{name}"),
                    "observed",
                    Some(format!(
                        "path:{}",
                        display_path(&resolved_path, &workspace_root)
                    )),
                    "relative path dependency from Cargo.toml",
                );
            }
        }
    }

    let lock_path = workspace_root.join("Cargo.lock");
    let locked_packages = match fs::read(&lock_path) {
        Ok(bytes) => {
            let (digest, complete) = digest_bytes(&bytes, 8 * 1024 * 1024);
            add_observation(
                &mut report,
                &format!("file:{}", display_path(&lock_path, &workspace_root)),
                "locked",
                Some(digest_value(&digest, complete)),
                "Cargo.lock content digest",
            );
            match std::str::from_utf8(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|text| toml::from_str::<toml::Value>(text).map_err(|e| e.to_string()))
            {
                Ok(value) => parse_locked_packages(&value),
                Err(parse_error) => {
                    add_unresolved(
                        &mut report,
                        "lockfile",
                        format!("malformed Cargo.lock: {}", first_line(&parse_error)),
                    );
                    Vec::new()
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            add_unresolved(&mut report, "lockfile", "Cargo.lock is missing".to_string());
            Vec::new()
        }
        Err(e) => {
            add_unresolved(
                &mut report,
                "lockfile",
                format!("could not read Cargo.lock: {e}"),
            );
            Vec::new()
        }
    };

    let mut all_package_names = BTreeSet::new();
    if let Some(name) = package_name {
        all_package_names.insert(name);
    }
    all_package_names.extend(declarations.keys().cloned());
    all_package_names.extend(locked_packages.iter().map(|package| package.name.clone()));
    for package in &locked_packages {
        let source = package
            .source
            .as_deref()
            .map(redact_url_credentials)
            .unwrap_or_else(|| "workspace-or-path".to_string());
        let checksum = package
            .checksum
            .as_deref()
            .map(|value| format!(" checksum={value}"))
            .unwrap_or_default();
        add_observation(
            &mut report,
            &format!("package:{}", package.name),
            "locked",
            Some(format!("{} source={source}{checksum}", package.version)),
            "package entry in Cargo.lock",
        );
    }
    for name in declarations.keys() {
        if !locked_packages.iter().any(|package| package.name == *name) {
            add_unresolved(
                &mut report,
                &format!("package:{name}"),
                "declared dependency has no matching Cargo.lock entry".to_string(),
            );
        }
    }

    if let Some(metadata) = &metadata {
        let mut names = Vec::new();
        for package in &metadata.packages {
            names.push(format!("{}@{}", package.name, package.version));
            add_observation(
                &mut report,
                &format!("package:{}", package.name),
                "observed",
                Some(package.version.clone()),
                "workspace package selected by cargo metadata --no-deps --offline",
            );
            let package_manifest = PathBuf::from(&package.manifest_path);
            if !package_manifest.starts_with(&workspace_root) {
                add_unresolved(
                    &mut report,
                    &format!("package:{}", package.name),
                    format!(
                        "workspace manifest is outside the detected workspace: {}",
                        display_path(&package_manifest, &workspace_root)
                    ),
                );
            }
        }
        names.sort();
        add_observation(
            &mut report,
            "workspace:packages",
            "observed",
            Some(names.join(",")),
            "Cargo metadata workspace package selection",
        );
    } else {
        add_unresolved(
            &mut report,
            "workspace:packages",
            "cargo metadata --offline could not resolve the workspace".to_string(),
        );
    }

    let cargo_home = cargo_home();
    if let Some(home) = cargo_home.as_ref() {
        add_observation(
            &mut report,
            "cargo-home",
            "observed",
            Some(if home.is_dir() {
                "present".to_string()
            } else {
                "missing".to_string()
            }),
            &format!(
                "Cargo home location {}",
                display_path(home, &workspace_root)
            ),
        );
        inspect_installed_bins(&mut report, home);
    } else {
        add_unresolved(
            &mut report,
            "cargo-home",
            "HOME and CARGO_HOME are unset".to_string(),
        );
    }

    let config_infos = inspect_configs(
        &mut report,
        manifest_parent,
        cargo_home.as_deref(),
        &workspace_root,
    );
    let target_dir = resolve_target_dir(&config_infos, &workspace_root);
    add_observation(
        &mut report,
        "target-dir",
        "observed",
        Some(display_path(&target_dir, &workspace_root)),
        "resolved from CARGO_TARGET_DIR, Cargo config, or the workspace target default",
    );
    inspect_target(
        &mut report,
        &target_dir,
        &all_package_names,
        &manifest,
        &lock_path,
        &config_infos,
        &workspace_root,
    );

    inspect_runtime(&mut report, manifest_parent);
    inspect_toolchains(&mut report, manifest_parent, &workspace_root);
    inspect_workspace_status(&mut report, &workspace_root);
    inspect_downloads(&mut report, cargo_home.as_deref(), &locked_packages);

    report.observations.sort_by(|left, right| {
        (&left.subject, &left.state, &left.value, &left.detail).cmp(&(
            &right.subject,
            &right.state,
            &right.value,
            &right.detail,
        ))
    });
    report.observations.dedup();
    report.warnings.sort();
    report.warnings.dedup();
    Ok(report)
}

fn empty_report(manifest: &Path, workspace_root: &Path) -> Report {
    Report {
        schema_version: 1,
        tool: "envdrift".to_string(),
        platform: env::consts::OS.to_string(),
        manifest_path: display_path(manifest, workspace_root),
        workspace_root: display_path(workspace_root, workspace_root),
        network_access_attempted: false,
        runtime: RuntimeIdentity {
            cargo: None,
            rustc: None,
            active_toolchain: None,
        },
        observations: Vec::new(),
        warnings: Vec::new(),
    }
}

fn locate_manifest(requested: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = requested {
        let path = if path.is_dir() {
            path.join("Cargo.toml")
        } else {
            path.to_path_buf()
        };
        return canonical_or_original(&path);
    }
    let current =
        env::current_dir().map_err(|e| error(format!("could not read current directory: {e}")))?;
    let result = Command::new("cargo")
        .args(["locate-project", "--message-format", "plain", "--offline"])
        .current_dir(&current)
        .output();
    if let Ok(output) = result {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return canonical_or_original(Path::new(&path));
            }
        }
    }
    let fallback = current.join("Cargo.toml");
    if fallback.is_file() {
        canonical_or_original(&fallback)
    } else {
        Err(error("could not locate Cargo.toml; pass --manifest-path"))
    }
}

fn canonical_or_original(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|e| error(format!("could not read current directory: {e}")))?
            .join(path)
    };
    if !path.is_file() {
        return Err(error(format!(
            "manifest does not exist: {}",
            path.display()
        )));
    }
    Ok(fs::canonicalize(&path).unwrap_or(path))
}

fn resolve_from_current_dir(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn run_cargo_metadata(manifest: &Path, current_dir: &Path) -> Option<CargoMetadata> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
        ])
        .arg(manifest)
        .current_dir(current_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn collect_declarations(value: &toml::Value) -> BTreeMap<String, DeclaredDependency> {
    let mut declarations = BTreeMap::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = value.get(section).and_then(toml::Value::as_table) {
            collect_dependency_table(&mut declarations, table, section);
        }
    }
    if let Some(table) = value
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        collect_dependency_table(&mut declarations, table, "workspace.dependencies");
    }
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for (target, target_value) in targets {
            if let Some(target_table) = target_value.as_table() {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(table) = target_table.get(section).and_then(toml::Value::as_table) {
                        collect_dependency_table(
                            &mut declarations,
                            table,
                            &format!("target.{target}.{section}"),
                        );
                    }
                }
            }
        }
    }
    declarations
}

fn collect_dependency_table(
    declarations: &mut BTreeMap<String, DeclaredDependency>,
    table: &toml::map::Map<String, toml::Value>,
    section: &str,
) {
    for (name, value) in table {
        let dependency =
            declarations
                .entry(name.to_string())
                .or_insert_with(|| DeclaredDependency {
                    specs: Vec::new(),
                    sections: BTreeSet::new(),
                    path: None,
                });
        dependency.specs.push(value_summary(value));
        dependency.sections.insert(section.to_string());
        if let Some(path) = value
            .as_table()
            .and_then(|table| table.get("path"))
            .and_then(toml::Value::as_str)
        {
            dependency.path = Some(PathBuf::from(path));
        }
    }
}

fn value_summary(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.to_string(),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        toml::Value::Boolean(value) => value.to_string(),
        toml::Value::Datetime(value) => value.to_string(),
        toml::Value::Array(values) => values
            .iter()
            .map(value_summary)
            .collect::<Vec<_>>()
            .join(","),
        toml::Value::Table(table) => {
            let keys = table.keys().cloned().collect::<Vec<_>>().join(",");
            format!("table({keys})")
        }
    }
}

fn parse_locked_packages(value: &toml::Value) -> Vec<LockedPackage> {
    let mut packages = Vec::new();
    if let Some(entries) = value.get("package").and_then(toml::Value::as_array) {
        for entry in entries {
            let Some(table) = entry.as_table() else {
                continue;
            };
            let Some(name) = table.get("name").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(version) = table.get("version").and_then(toml::Value::as_str) else {
                continue;
            };
            packages.push(LockedPackage {
                name: name.to_string(),
                version: version.to_string(),
                source: table
                    .get("source")
                    .and_then(toml::Value::as_str)
                    .map(ToString::to_string),
                checksum: table
                    .get("checksum")
                    .and_then(toml::Value::as_str)
                    .map(ToString::to_string),
            });
        }
    }
    packages.sort_by(|left, right| {
        (&left.name, &left.version, &left.source).cmp(&(&right.name, &right.version, &right.source))
    });
    packages
}

fn cargo_home() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(path));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo"))
}

fn config_paths(start: &Path, cargo_home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut current = Some(start);
    while let Some(directory) = current {
        for name in ["config.toml", "config"] {
            let path = directory.join(".cargo").join(name);
            if path.is_file() {
                paths.push(path);
            }
        }
        current = directory.parent();
    }
    if let Some(home) = cargo_home {
        for name in ["config.toml", "config"] {
            let path = home.join(name);
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    let mut unique = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(key) {
            unique.push(path);
        }
    }
    unique
}

fn inspect_configs(
    report: &mut Report,
    start: &Path,
    cargo_home: Option<&Path>,
    workspace_root: &Path,
) -> Vec<ConfigInfo> {
    let mut infos = Vec::new();
    for path in config_paths(start, cargo_home) {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                add_unresolved(
                    report,
                    &format!("config:{}", display_path(&path, workspace_root)),
                    format!("could not read Cargo config: {e}"),
                );
                continue;
            }
        };
        let (digest, complete) = digest_bytes(&bytes, 8 * 1024 * 1024);
        let subject = format!("config:{}", display_path(&path, workspace_root));
        add_observation(
            report,
            &subject,
            "declared",
            Some(digest_value(&digest, complete)),
            "Cargo configuration file discovered by hierarchical search",
        );
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<toml::Value>(text).map_err(|e| e.to_string()));
        let value = match parsed {
            Ok(value) => value,
            Err(parse_error) => {
                add_unresolved(
                    report,
                    &subject,
                    format!("malformed Cargo config: {}", first_line(&parse_error)),
                );
                continue;
            }
        };
        let target_dir = value
            .get("build")
            .and_then(|build| build.get("target-dir"))
            .and_then(toml::Value::as_str)
            .map(ToString::to_string);
        let mut details = Vec::new();
        flatten_config(&value, "", &mut details, 64);
        details.retain(|detail| {
            detail.starts_with("source.")
                || detail.starts_with("registries.")
                || detail == "build.target-dir"
                || detail == "build.rustc-wrapper"
        });
        details.sort();
        for detail in &details {
            add_observation(
                report,
                &subject,
                "observed",
                Some(detail.clone()),
                "selected Cargo config key; sensitive values are omitted",
            );
        }
        infos.push(ConfigInfo { path, target_dir });
    }
    infos
}

fn flatten_config(value: &toml::Value, prefix: &str, output: &mut Vec<String>, limit: usize) {
    if output.len() >= limit {
        return;
    }
    if let Some(table) = value.as_table() {
        for (key, child) in table {
            let full_key = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            if child.is_table() {
                flatten_config(child, &full_key, output, limit);
            } else {
                let value = if sensitive_name(key) {
                    "<omitted>".to_string()
                } else {
                    redact_url_credentials(&value_summary(child))
                };
                output.push(format!("{full_key}={value}"));
            }
            if output.len() >= limit {
                return;
            }
        }
    }
}

fn resolve_target_dir(configs: &[ConfigInfo], workspace_root: &Path) -> PathBuf {
    if let Some(value) = env::var_os("CARGO_TARGET_DIR") {
        return resolve_path_value(&PathBuf::from(value), workspace_root);
    }
    if let Some(value) = env::var_os("CARGO_BUILD_TARGET_DIR") {
        return resolve_path_value(&PathBuf::from(value), workspace_root);
    }
    for config in configs {
        if let Some(value) = config.target_dir.as_deref() {
            let raw = PathBuf::from(value);
            return if raw.is_absolute() {
                raw
            } else {
                config.path.parent().unwrap_or(workspace_root).join(raw)
            };
        }
    }
    workspace_root.join("target")
}

fn resolve_path_value(path: &Path, relative_to: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        relative_to.join(path)
    }
}

fn inspect_target(
    report: &mut Report,
    target_dir: &Path,
    package_names: &BTreeSet<String>,
    manifest: &Path,
    lock_path: &Path,
    configs: &[ConfigInfo],
    workspace_root: &Path,
) {
    if !target_dir.is_dir() {
        add_observation(
            report,
            "target:metadata",
            "observed",
            Some("missing".to_string()),
            "target directory is not present",
        );
        for name in package_names {
            add_observation(
                report,
                &format!("package:{name}"),
                "built",
                Some("not-built".to_string()),
                "no target directory was observed",
            );
        }
        return;
    }
    let file_count = count_files(target_dir, 50_000);
    let fingerprint_count = count_files(&target_dir.join(".fingerprint"), 50_000);
    add_observation(
        report,
        "target:metadata",
        "observed",
        Some(format!(
            "files={file_count};fingerprint_files={fingerprint_count}"
        )),
        "bounded target metadata inventory",
    );
    if file_count == 0 {
        add_observation(
            report,
            "target:staleness",
            "observed",
            Some("empty".to_string()),
            "target directory contains no files",
        );
    }
    for name in package_names {
        let built = artifact_exists(target_dir, name);
        add_observation(
            report,
            &format!("package:{name}"),
            "built",
            Some(if built { "present" } else { "not-observed" }.to_string()),
            "bounded search of debug and release dependency artifacts",
        );
    }

    let newest_input = newest_mtime(
        [
            Some(manifest.to_path_buf()),
            lock_path.is_file().then(|| lock_path.to_path_buf()),
        ]
        .into_iter()
        .flatten()
        .chain(configs.iter().map(|config| config.path.clone())),
    );
    let newest_target = newest_mtime_from_dir(target_dir, 50_000);
    match (newest_input, newest_target) {
        (Some(input), Some(target)) if target < input => {
            add_observation(
                report,
                "target:staleness",
                "observed",
                Some("stale".to_string()),
                "target metadata predates a declaration or configuration file",
            );
            add_unresolved(
                report,
                "target:staleness",
                "target output may be stale; rebuild evidence is required".to_string(),
            );
        }
        (Some(_), Some(_)) => add_observation(
            report,
            "target:staleness",
            "observed",
            Some("not-stale".to_string()),
            "target metadata is not older than declaration files",
        ),
        _ => add_observation(
            report,
            "target:staleness",
            "unresolved",
            None,
            "target timestamps could not be compared",
        ),
    }
    let _ = workspace_root;
}

fn inspect_downloads(report: &mut Report, cargo_home: Option<&Path>, packages: &[LockedPackage]) {
    for package in packages {
        let Some(source) = package.source.as_deref() else {
            continue;
        };
        let subject = format!("package:{}", package.name);
        let (value, detail) = match cargo_home {
            Some(home) if source.starts_with("registry+") || source.contains("crates.io") => {
                let needle = format!("{}-{}.crate", package.name, package.version);
                let present = walk_for_name(&home.join("registry/cache"), &needle, 4, 20_000);
                if present {
                    ("present", "matching registry archive in Cargo cache")
                } else {
                    (
                        "missing",
                        "matching registry archive was not found in Cargo cache",
                    )
                }
            }
            Some(home) if source.starts_with("git+") => {
                let present = home.join("git").join("checkouts").is_dir()
                    || home.join("git").join("db").is_dir();
                if present {
                    (
                        "cache-present",
                        "git cache root observed; exact checkout mapping is unresolved",
                    )
                } else {
                    ("missing", "git cache root was not found")
                }
            }
            Some(_) => (
                "unresolved",
                "source type is not mapped by the Cargo-only MVP",
            ),
            None => ("unresolved", "Cargo home is unavailable"),
        };
        add_observation(
            report,
            &subject,
            "downloaded",
            Some(value.to_string()),
            detail,
        );
        if value == "missing" || value == "unresolved" {
            add_unresolved(
                report,
                &subject,
                format!("download state is {value}: {detail}"),
            );
        }
    }
}

fn inspect_installed_bins(report: &mut Report, cargo_home: &Path) {
    let bin_dir = cargo_home.join("bin");
    if !bin_dir.is_dir() {
        add_observation(
            report,
            "installed:cargo-bin",
            "installed",
            Some("none-observed".to_string()),
            "Cargo home bin directory is missing",
        );
        return;
    }
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(&bin_dir) {
        for entry in entries.flatten() {
            if entry
                .file_type()
                .map(|kind| kind.is_file() || kind.is_symlink())
                .unwrap_or(false)
            {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    for name in names {
        add_observation(
            report,
            &format!("installed:cargo-bin:{name}"),
            "installed",
            Some("present".to_string()),
            "executable observed under CARGO_HOME/bin",
        );
    }
}

fn inspect_runtime(report: &mut Report, current_dir: &Path) {
    let cargo = command_first_line("cargo", &["--version", "--verbose"], current_dir);
    let rustc = command_first_line("rustc", &["--version", "--verbose"], current_dir);
    let active_toolchain = command_first_line("rustup", &["show", "active-toolchain"], current_dir);
    report.runtime = RuntimeIdentity {
        cargo: cargo.clone(),
        rustc: rustc.clone(),
        active_toolchain: active_toolchain.clone(),
    };
    for (subject, value, detail) in [
        ("tool:cargo", cargo, "installed Cargo identity"),
        ("tool:rustc", rustc, "installed rustc identity"),
        ("toolchain", active_toolchain, "active rustup toolchain"),
    ] {
        if let Some(value) = value {
            add_observation(report, subject, "installed", Some(value), detail);
        } else {
            add_unresolved(report, subject, format!("could not observe {detail}"));
        }
    }
}

fn inspect_toolchains(report: &mut Report, start: &Path, workspace_root: &Path) {
    let files = find_toolchain_files(start);
    let selected = files.first().cloned();
    for file in &files {
        let label = display_path(&file.path, workspace_root);
        add_observation(
            report,
            &format!("toolchain-file:{label}"),
            "declared",
            Some(file.digest.clone()),
            "rust-toolchain file discovered by nearest-directory search",
        );
        match fs::read_to_string(&file.path).and_then(|text| {
            parse_toolchain_summary(&file.path, &text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }) {
            Ok(summary) => add_observation(
                report,
                &format!("toolchain-file:{label}"),
                "observed",
                Some(summary),
                "toolchain declaration",
            ),
            Err(e) => add_unresolved(
                report,
                &format!("toolchain-file:{label}"),
                format!("malformed toolchain file: {e}"),
            ),
        }
    }
    if let Some(file) = selected {
        add_observation(
            report,
            "toolchain-selection",
            "observed",
            Some(display_path(&file.path, workspace_root)),
            "nearest rust-toolchain file selected; rust-toolchain takes precedence over rust-toolchain.toml in one directory",
        );
    } else {
        add_observation(
            report,
            "toolchain-selection",
            "observed",
            Some("rustup-default".to_string()),
            "no repository toolchain file was observed",
        );
    }
}

fn find_toolchain_files(start: &Path) -> Vec<ToolchainFile> {
    let mut files = Vec::new();
    let mut current = Some(start);
    while let Some(directory) = current {
        let legacy = directory.join("rust-toolchain");
        let toml = directory.join("rust-toolchain.toml");
        let candidates = if legacy.is_file() {
            vec![legacy]
        } else if toml.is_file() {
            vec![toml]
        } else {
            Vec::new()
        };
        for path in candidates {
            if let Ok(bytes) = fs::read(&path) {
                let (digest, complete) = digest_bytes(&bytes, 1024 * 1024);
                files.push(ToolchainFile {
                    path,
                    digest: digest_value(&digest, complete),
                });
            }
        }
        current = directory.parent();
    }
    files
}

fn parse_toolchain_summary(path: &Path, text: &str) -> std::result::Result<String, String> {
    if path.file_name().and_then(|name| name.to_str()) == Some("rust-toolchain.toml") {
        let value = toml::from_str::<toml::Value>(text).map_err(|e| e.to_string())?;
        let table = value
            .get("toolchain")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| "[toolchain] section is missing".to_string())?;
        let channel = table
            .get("channel")
            .and_then(toml::Value::as_str)
            .map(ToString::to_string);
        let path_value = table
            .get("path")
            .and_then(toml::Value::as_str)
            .map(ToString::to_string);
        match (channel, path_value) {
            (Some(channel), None) => Ok(format!("channel={channel}")),
            (None, Some(path)) => Ok(format!("path={}", redact_absolute_value(&path))),
            (Some(_), Some(_)) => Err("channel and path are mutually exclusive".to_string()),
            (None, None) => Err("toolchain requires channel or path".to_string()),
        }
    } else {
        let value = text.trim();
        if value.is_empty() {
            Err("legacy toolchain file is empty".to_string())
        } else {
            Ok(format!("channel={}", first_line(value)))
        }
    }
}

fn inspect_workspace_status(report: &mut Report, workspace_root: &Path) {
    let output = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=normal",
        ])
        .current_dir(workspace_root)
        .output();
    let Ok(output) = output else {
        add_unresolved(
            report,
            "workspace:status",
            "git status could not be executed".to_string(),
        );
        return;
    };
    if !output.status.success() {
        add_observation(
            report,
            "workspace:status",
            "observed",
            Some("not-a-git-worktree".to_string()),
            "git status did not identify a repository",
        );
        return;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let branch = lines.next().unwrap_or("unknown").trim_start_matches("## ");
    let changed = lines.count();
    add_observation(
        report,
        "workspace:status",
        "observed",
        Some(if changed == 0 { "clean" } else { "dirty" }.to_string()),
        &format!("git branch={branch}; changed_entries={changed}"),
    );
}

fn command_first_line(program: &str, args: &[&str], current_dir: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(current_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}

fn artifact_exists(target_dir: &Path, package_name: &str) -> bool {
    let names = [package_name.to_string(), package_name.replace('-', "_")];
    for directory in [
        target_dir.join("debug/deps"),
        target_dir.join("release/deps"),
    ] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let Some(file_name) = entry.file_name().to_str().map(ToString::to_string) else {
                continue;
            };
            if names.iter().any(|name| {
                file_name.starts_with(&format!("{name}-"))
                    || file_name.starts_with(&format!("lib{name}-"))
            }) {
                return true;
            }
        }
    }
    false
}

fn walk_for_name(root: &Path, wanted: &str, max_depth: usize, max_entries: usize) -> bool {
    if !root.is_dir() {
        return false;
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut entries_seen = 0usize;
    while let Some((directory, depth)) = stack.pop() {
        if depth > max_depth || entries_seen >= max_entries {
            break;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            entries_seen += 1;
            if entry.file_name() == wanted {
                return true;
            }
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                stack.push((entry.path(), depth + 1));
            }
            if entries_seen >= max_entries {
                break;
            }
        }
    }
    false
}

fn count_files(root: &Path, max_entries: usize) -> usize {
    if !root.is_dir() {
        return 0;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut count = 0usize;
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                stack.push(entry.path());
            } else {
                count += 1;
                if count >= max_entries {
                    return count;
                }
            }
        }
    }
    count
}

fn newest_mtime<I>(paths: I) -> Option<SystemTime>
where
    I: IntoIterator<Item = PathBuf>,
{
    paths
        .into_iter()
        .filter_map(|path| {
            fs::metadata(path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
        })
        .max()
}

fn newest_mtime_from_dir(root: &Path, max_entries: usize) -> Option<SystemTime> {
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut latest = None;
    let mut seen = 0usize;
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    stack.push(entry.path());
                } else if let Ok(modified) = metadata.modified() {
                    latest = latest.max(Some(modified));
                }
            }
            if seen >= max_entries {
                return latest;
            }
        }
    }
    latest
}

fn digest_bytes(bytes: &[u8], limit: usize) -> (String, bool) {
    let complete = bytes.len() <= limit;
    let mut hasher = Sha256::new();
    hasher.update(&bytes[..bytes.len().min(limit)]);
    (hex_digest(hasher.finalize()), complete)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn digest_value(digest: &str, complete: bool) -> String {
    if complete {
        format!("sha256:{digest}")
    } else {
        format!("sha256-prefix:{digest}")
    }
}

fn display_path(path: &Path, workspace_root: &Path) -> String {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if path == workspace_root {
        return "<workspace>".to_string();
    }
    if let Ok(relative) = path.strip_prefix(workspace_root) {
        let relative = relative.to_string_lossy();
        return format!("<workspace>/{relative}");
    }
    let (digest, _) = digest_bytes(path.to_string_lossy().as_bytes(), 4096);
    format!("<external>/{digest}")
}

fn redact_url_credentials(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_string();
    };
    let authority_start = scheme_end + 3;
    let Some(at_offset) = value[authority_start..].find('@') else {
        return value.to_string();
    };
    format!(
        "{}{}",
        &value[..authority_start],
        &value[authority_start + at_offset + 1..]
    )
}

fn redact_absolute_value(value: &str) -> String {
    let path = Path::new(value);
    if path.is_absolute() {
        let (digest, _) = digest_bytes(value.as_bytes(), 4096);
        format!("<external>/{digest}")
    } else {
        value.to_string()
    }
}

fn sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "token",
        "password",
        "passwd",
        "secret",
        "private",
        "credential",
        "auth",
        "cookie",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

fn add_observation(
    report: &mut Report,
    subject: &str,
    state: &str,
    value: Option<String>,
    detail: &str,
) {
    report.observations.push(Observation {
        subject: subject.to_string(),
        state: state.to_string(),
        value,
        detail: detail.to_string(),
    });
}

fn add_unresolved(report: &mut Report, subject: &str, detail: String) {
    add_observation(report, subject, "unresolved", None, &detail);
    report.warnings.push(detail);
}

fn join_set(values: &BTreeSet<String>) -> String {
    values.iter().cloned().collect::<Vec<_>>().join(",")
}

fn first_line(value: &str) -> String {
    value.lines().next().unwrap_or(value).trim().to_string()
}

fn render_report(report: &Report, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(report)
            .map(|json| format!("{json}\n"))
            .map_err(|e| error(format!("could not serialize report: {e}"))),
        OutputFormat::Text => Ok(render_text(report)),
    }
}

fn render_text(report: &Report) -> String {
    let mut output = String::new();
    output.push_str("envdrift scan\n");
    output.push_str(&format!("manifest: {}\n", report.manifest_path));
    output.push_str(&format!("workspace: {}\n", report.workspace_root));
    output.push_str("network_access_attempted: false\n");
    output.push_str("observations:\n");
    for observation in &report.observations {
        let value = observation.value.as_deref().unwrap_or("-");
        output.push_str(&format!(
            "- {} [{}] {}: {}\n",
            observation.state, observation.subject, value, observation.detail
        ));
    }
    if !report.warnings.is_empty() {
        output.push_str("unresolved:\n");
        for warning in &report.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }
    output
}

pub fn explain_reports(baseline: &Report, current: &Report) -> String {
    let mut baseline_map = BTreeMap::new();
    for observation in &baseline.observations {
        baseline_map.insert(
            (observation.subject.clone(), observation.state.clone()),
            observation,
        );
    }
    let mut current_map = BTreeMap::new();
    for observation in &current.observations {
        current_map.insert(
            (observation.subject.clone(), observation.state.clone()),
            observation,
        );
    }
    let keys = baseline_map
        .keys()
        .chain(current_map.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut changes = Vec::new();
    for key in keys {
        match (baseline_map.get(&key), current_map.get(&key)) {
            (Some(before), Some(after))
                if before.value == after.value && before.detail == after.detail => {}
            (Some(before), Some(after)) => changes.push(format!(
                "- [{}] {}: {} -> {}",
                key.1,
                key.0,
                observation_value(before),
                observation_value(after)
            )),
            (None, Some(after)) => changes.push(format!(
                "- [{}] {}: added {}",
                key.1,
                key.0,
                observation_value(after)
            )),
            (Some(before), None) => changes.push(format!(
                "- [{}] {}: removed {}",
                key.1,
                key.0,
                observation_value(before)
            )),
            (None, None) => {}
        }
    }
    let mut output = String::new();
    output.push_str("envdrift explanation\n");
    output.push_str(&format!("baseline: {}\n", baseline.manifest_path));
    output.push_str(&format!("current: {}\n", current.manifest_path));
    output.push_str("changes:\n");
    if changes.is_empty() {
        output.push_str("- none observed\n");
    } else {
        for change in changes {
            output.push_str(&format!("{change}\n"));
        }
    }
    if !current.warnings.is_empty() {
        output.push_str("unresolved:\n");
        for warning in &current.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }
    output
}

fn explain_without_baseline(report: &Report, path: &Path) -> String {
    let mut output = String::new();
    output.push_str("envdrift explanation\n");
    output.push_str("baseline: missing\n");
    output.push_str(&format!("current: {}\n", report.manifest_path));
    output.push_str("changes:\n- none comparable\n");
    output.push_str("unresolved:\n");
    output.push_str(&format!("- baseline file is missing: {}\n", path.display()));
    output.push_str("- create one with envdrift export --format json --output ");
    output.push_str(&format!("{}\n", path.display()));
    output
}

fn observation_value(observation: &Observation) -> String {
    let value = observation.value.as_deref().unwrap_or("-");
    format!("{value} ({})", observation.detail)
}

fn emit(text: String, output: Option<&Path>) -> Result<()> {
    if let Some(path) = output {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|e| error(format!("could not create {}: {e}", parent.display())))?;
        }
        fs::write(path, text.as_bytes())
            .map_err(|e| error(format!("could not write {}: {e}", path.display())))?;
        let written = fs::read(path)
            .map_err(|e| error(format!("could not verify {}: {e}", path.display())))?;
        if written != text.as_bytes() {
            return Err(error(format!(
                "read-back verification failed for {}",
                path.display()
            )));
        }
    } else {
        print!("{text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_config_names_are_redacted() {
        assert!(sensitive_name("registry-token"));
        assert!(sensitive_name("password"));
        assert!(!sensitive_name("target-dir"));
    }

    #[test]
    fn url_credentials_are_removed() {
        assert_eq!(
            redact_url_credentials("https://user:secret@example.invalid/repo"),
            "https://example.invalid/repo"
        );
    }

    #[test]
    fn path_dependency_is_explicit() {
        let value =
            toml::from_str::<toml::Value>("[dependencies]\nlocal = { path = \"../local\" }\n")
                .expect("valid TOML");
        let declarations = collect_declarations(&value);
        assert_eq!(declarations["local"].path, Some(PathBuf::from("../local")));
    }

    #[test]
    fn explanation_is_stable() {
        let report = Report {
            schema_version: 1,
            tool: "envdrift".to_string(),
            platform: "linux".to_string(),
            manifest_path: "<workspace>/Cargo.toml".to_string(),
            workspace_root: "<workspace>".to_string(),
            network_access_attempted: false,
            runtime: RuntimeIdentity {
                cargo: None,
                rustc: None,
                active_toolchain: None,
            },
            observations: vec![Observation {
                subject: "tool:cargo".to_string(),
                state: "installed".to_string(),
                value: Some("cargo 1".to_string()),
                detail: "identity".to_string(),
            }],
            warnings: Vec::new(),
        };
        assert!(explain_reports(&report, &report).contains("none observed"));
    }
}
