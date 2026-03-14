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
fn help_flag_returns_usage_output() {
    let directory = tempdir().expect("tempdir");
    let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

    let output = run_from_args(["atom", "--help"], &root).expect("help should succeed");

    assert_eq!(output.exit_code, 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: atom"));
    assert!(output.stderr.is_empty());
}
