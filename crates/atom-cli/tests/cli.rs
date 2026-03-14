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
fn new_creates_a_minimal_bootable_project_scaffold() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let output = run_from_args(["atom", "new", "my_app"], &root).expect("scaffold should succeed");
    let project_root = root.join("my_app");

    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 output"),
        "Creating my_app...\nDone! Run `cd my_app && atom run --platform ios --target //apps/my_app:my_app` to get started.\n"
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
    assert!(project_root.join("platforms/BUILD.bazel").exists());
    assert!(project_root.join("apps/my_app/BUILD.bazel").exists());
    assert!(project_root.join("apps/my_app/src/lib.rs").exists());
    assert!(!project_root.join("MODULE.bazel.lock").exists());

    let module_bazel = fs::read_to_string(project_root.join("MODULE.bazel")).expect("module");
    assert!(module_bazel.contains("module_name = \"atom\""));
    assert!(module_bazel.contains("remote = \"https://github.com/alexjameslittle/atom.git\""));
    assert!(module_bazel.contains("branch = \"main\""));
    assert!(module_bazel.contains("name = \"app_crates\""));
    assert!(module_bazel.contains("package = \"camino\""));
    assert!(module_bazel.contains("name = \"rules_apple\""));
    assert!(module_bazel.contains("name = \"rules_swift\""));
    assert!(module_bazel.contains("name = \"platforms\""));
    assert!(module_bazel.contains("android_sdk_repository_extension"));

    let build_bazel = fs::read_to_string(project_root.join("BUILD.bazel")).expect("build file");
    assert!(build_bazel.contains("actual = \"@atom//:atom\""));

    let platforms_build =
        fs::read_to_string(project_root.join("platforms/BUILD.bazel")).expect("platforms");
    assert!(platforms_build.contains("name = \"arm64-v8a\""));
    assert!(platforms_build.contains("@platforms//os:android"));
    assert!(platforms_build.contains("@platforms//cpu:arm64"));

    let app_build = fs::read_to_string(project_root.join("apps/my_app/BUILD.bazel")).expect("app");
    assert!(app_build.contains("load(\"@atom//bzl/atom:defs.bzl\", \"atom_app\")"));
    assert!(app_build.contains("crate_name = \"my_app\""));
    assert!(app_build.contains("app_name = \"My App\""));
    assert!(app_build.contains("ios_bundle_id = \"com.example.my_app\""));
    assert!(app_build.contains("ios_deployment_target = \"18.0\""));
    assert!(app_build.contains("android_application_id = \"com.example.my_app\""));
    assert!(app_build.contains("android_min_sdk = 24"));
    assert!(app_build.contains("android_target_sdk = 35"));
    assert!(app_build.contains("\"@atom//crates/atom-runtime\""));

    let app_lib = fs::read_to_string(project_root.join("apps/my_app/src/lib.rs")).expect("lib");
    assert!(app_lib.contains("use atom_runtime::RuntimeConfig;"));
    assert!(app_lib.contains("pub fn atom_runtime_config() -> RuntimeConfig"));
    assert!(app_lib.contains("RuntimeConfig::builder().build()"));

    let mise_toml = fs::read_to_string(project_root.join("mise.toml")).expect("mise");
    assert!(mise_toml.contains("bazel = \"8.4.2\""));
    assert!(mise_toml.contains("rust = \"1.92.0\""));
    assert!(mise_toml.contains("java = \"temurin-21\""));

    let readme = fs::read_to_string(project_root.join("README.md")).expect("readme");
    assert!(readme.contains("//apps/my_app:my_app"));
    assert!(readme.contains("minimal bootable app crate"));
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
fn new_requires_a_name_when_no_interactive_is_set() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let error = run_from_args(["atom", "new", "--no-interactive"], &root)
        .expect_err("missing name should fail");

    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    assert!(
        error
            .message
            .contains("project name is required when --no-interactive is set")
    );
}

#[test]
fn new_requires_a_name_when_not_attached_to_a_tty() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let error = run_from_args(["atom", "new"], &root).expect_err("missing name should fail");

    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    assert!(
        error
            .message
            .contains("project name is required when not attached to an interactive terminal")
    );
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
