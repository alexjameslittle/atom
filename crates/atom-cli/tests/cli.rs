use atom_cli::run_from_args;
use camino::Utf8PathBuf;
use std::fs;
use tempfile::tempdir;

#[test]
fn prebuild_requires_a_target_flag() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let error = run_from_args(["atom", "prebuild"], &root).expect_err("missing target should fail");
    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
}

#[test]
fn run_requires_a_target_flag() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let error = run_from_args(["atom", "run", "--platform", "ios"], &root)
        .expect_err("missing target should fail");
    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
}

#[test]
fn run_accepts_a_device_flag_after_platform_flag() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
    fs::write(root.join("MODULE.bazel"), "module(name = \"atom\")\n").expect("workspace");

    let error = run_from_args(
        [
            "atom",
            "run",
            "--platform",
            "ios",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--device",
            "SIM-123",
        ],
        &root,
    )
    .expect_err("missing workspace should fail after clap accepts the command");

    assert_ne!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
}

#[test]
fn new_creates_the_minimal_project_scaffold() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let output = run_from_args(["atom", "new", "my_app"], &root).expect("scaffold should succeed");
    let project_root = root.join("my_app");

    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 output"),
        format!("{project_root}\n")
    );
    assert!(project_root.is_dir());
    assert!(project_root.join("MODULE.bazel").exists());
    assert_eq!(
        fs::read_to_string(project_root.join(".bazelversion")).expect("bazelversion"),
        "8.4.2\n"
    );
    assert!(project_root.join(".bazelrc").exists());
    assert!(project_root.join("mise.toml").exists());
    assert!(project_root.join("BUILD.bazel").exists());
    assert!(project_root.join("README.md").exists());
    assert!(project_root.join(".gitignore").exists());
    assert!(!project_root.join("MODULE.bazel.lock").exists());

    let module_bazel = fs::read_to_string(project_root.join("MODULE.bazel")).expect("module");
    assert!(module_bazel.contains("module_name = \"atom\""));
    assert!(module_bazel.contains("remote = \"https://github.com/alexjameslittle/atom.git\""));
    assert!(module_bazel.contains("branch = \"main\""));
    assert!(module_bazel.contains("crate.from_specs(name = \"app_crates\")"));
    assert!(module_bazel.contains("package = \"camino\""));

    let build_bazel = fs::read_to_string(project_root.join("BUILD.bazel")).expect("build file");
    assert!(build_bazel.contains("actual = \"@atom//:atom\""));

    let mise_toml = fs::read_to_string(project_root.join("mise.toml")).expect("mise");
    assert!(mise_toml.contains("bazel = \"8.4.2\""));
    assert!(mise_toml.contains("rust = \"1.92.0\""));
    assert!(mise_toml.contains("java = \"temurin-21\""));
}

#[test]
fn new_rejects_an_existing_directory() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
    fs::create_dir(root.join("my_app")).expect("existing directory");

    let error = run_from_args(["atom", "new", "my_app"], &root)
        .expect_err("existing directory should fail");

    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    assert_eq!(error.path.as_deref(), Some(root.join("my_app").as_str()));
    assert!(error.message.contains("already exists"));
}

#[test]
fn new_rejects_invalid_project_names() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let error = run_from_args(["atom", "new", "My-Invalid-Name!"], &root)
        .expect_err("invalid project name should fail");

    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    assert!(error.message.contains("lowercase ASCII letters"));
}

#[test]
fn help_flag_returns_usage_output() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let output = run_from_args(["atom", "--help"], &root).expect("help should succeed");

    assert_eq!(output.exit_code, 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: atom"));
    assert!(output.stderr.is_empty());
}
