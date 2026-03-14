use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};
use minijinja::context;

use crate::templates;
use crate::{
    ATOM_BUILD_BAZEL_VERSION, ATOM_FRAMEWORK_VERSION, ATOM_JAVA_RUNTIME_VERSION,
    ATOM_MISE_BAZELISK_VERSION, ATOM_MISE_JAVA_VERSION, ATOM_MISE_RUST_TOOLCHAIN_VERSION,
    ATOM_RULES_RUST_VERSION,
};

const FRAMEWORK_GIT_REMOTE: &str = "https://github.com/alexjameslittle/atom.git";
const FRAMEWORK_GIT_BRANCH: &str = "main";
const PLACEHOLDER_CRATE_PACKAGE: &str = "camino";
const PLACEHOLDER_CRATE_VERSION: &str = "=1.2.2";
const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

const SCAFFOLD_FILES: &[(&str, &str)] = &[
    ("MODULE.bazel", "project/MODULE.bazel"),
    (".bazelversion", "project/.bazelversion"),
    (".bazelrc", "project/.bazelrc"),
    ("mise.toml", "project/mise.toml"),
    ("BUILD.bazel", "project/BUILD.bazel"),
    ("README.md", "project/README.md"),
    (".gitignore", "project/.gitignore"),
];

pub(crate) fn scaffold_project(cwd: &Utf8Path, name: &str) -> AtomResult<Utf8PathBuf> {
    validate_project_name(name)?;

    let project_root = cwd.join(name);
    if project_root.exists() {
        return Err(AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("refusing to scaffold `{name}` because the target directory already exists"),
            project_root.as_str(),
        ));
    }

    fs::create_dir(&project_root).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to create project directory: {error}"),
            project_root.as_str(),
        )
    })?;

    let context = context! {
        project_name => name,
        framework_version => ATOM_FRAMEWORK_VERSION,
        framework_git_remote => FRAMEWORK_GIT_REMOTE,
        framework_git_branch => FRAMEWORK_GIT_BRANCH,
        bazel_version => ATOM_BUILD_BAZEL_VERSION,
        bazelisk_version => ATOM_MISE_BAZELISK_VERSION,
        rust_version => ATOM_MISE_RUST_TOOLCHAIN_VERSION,
        java_version => ATOM_MISE_JAVA_VERSION,
        java_runtime_version => ATOM_JAVA_RUNTIME_VERSION,
        rules_rust_version => ATOM_RULES_RUST_VERSION,
        placeholder_crate_package => PLACEHOLDER_CRATE_PACKAGE,
        placeholder_crate_version => PLACEHOLDER_CRATE_VERSION,
    };

    for &(relative_path, template_name) in SCAFFOLD_FILES {
        let contents = templates::render(template_name, context.clone())?;
        write_file(&project_root.join(relative_path), &contents)?;
    }

    Ok(project_root)
}

fn validate_project_name(name: &str) -> AtomResult<()> {
    if !is_valid_project_name(name) {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "project name must start with a lowercase ASCII letter and contain only lowercase ASCII letters, digits, and underscores",
        ));
    }

    if RUST_KEYWORDS.contains(&name) {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            format!("project name `{name}` is reserved in Rust; choose a different crate name"),
        ));
    }

    Ok(())
}

fn is_valid_project_name(name: &str) -> bool {
    let mut characters = name.chars();
    match characters.next() {
        Some(character) if character.is_ascii_lowercase() => (),
        _ => return false,
    }

    characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    })
}

fn write_file(path: &Utf8Path, contents: &str) -> AtomResult<()> {
    fs::write(path, contents).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to write scaffold file: {error}"),
            path.as_str(),
        )
    })
}

#[cfg(test)]
mod tests {
    use atom_ffi::AtomErrorCode;
    use camino::Utf8PathBuf;

    use super::{scaffold_project, validate_project_name};

    #[test]
    fn validate_project_name_rejects_rust_keywords() {
        let error = validate_project_name("crate").expect_err("keywords should fail");

        assert_eq!(error.code, AtomErrorCode::CliUsageError);
        assert!(error.message.contains("reserved in Rust"));
    }

    #[test]
    fn scaffold_project_renders_embedded_templates() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

        let project_root = scaffold_project(&root, "my_app").expect("scaffold should succeed");

        assert_eq!(project_root, root.join("my_app"));
        assert_eq!(
            std::fs::read_to_string(project_root.join(".bazelversion")).expect("bazelversion"),
            format!("{}\n", crate::ATOM_BUILD_BAZEL_VERSION)
        );

        let module_bazel =
            std::fs::read_to_string(project_root.join("MODULE.bazel")).expect("module");
        assert!(module_bazel.contains("module_name = \"atom\""));
        assert!(module_bazel.contains("package = \"camino\""));
        assert!(module_bazel.contains("version = \"=1.2.2\""));
    }
}
