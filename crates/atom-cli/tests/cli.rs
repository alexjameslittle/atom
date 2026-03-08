use atom_cli::run_from_args;
use camino::Utf8PathBuf;
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

    let error =
        run_from_args(["atom", "run", "ios"], &root).expect_err("missing target should fail");
    assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
}
