use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::FRAMEWORK_VERSION;
use camino::{Utf8Path, Utf8PathBuf};

const ROOT_BAZEL_RC: &str = include_str!("../../../.bazelrc");
const ROOT_BAZEL_VERSION: &str = include_str!("../../../.bazelversion");
const ROOT_MISE_TOML: &str = include_str!("../../../mise.toml");
const ROOT_MODULE_BAZEL: &str = include_str!("../../../MODULE.bazel");

const FRAMEWORK_GIT_REMOTE: &str = "https://github.com/alexjameslittle/atom.git";
const FRAMEWORK_GIT_BRANCH: &str = "main";
const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
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

    let scaffold = ProjectScaffold::for_name(name)?;

    fs::create_dir(&project_root).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to create project directory: {error}"),
            project_root.as_str(),
        )
    })?;

    write_file(&project_root.join("MODULE.bazel"), &scaffold.module_bazel)?;
    write_file(&project_root.join(".bazelversion"), &scaffold.bazelversion)?;
    write_file(&project_root.join(".bazelrc"), &scaffold.bazelrc)?;
    write_file(&project_root.join("mise.toml"), &scaffold.mise_toml)?;
    write_file(&project_root.join("BUILD.bazel"), BUILD_BAZEL_TEMPLATE)?;
    write_file(&project_root.join("README.md"), &scaffold.readme)?;
    write_file(&project_root.join(".gitignore"), GITIGNORE_TEMPLATE)?;

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

struct ProjectScaffold {
    module_bazel: String,
    bazelversion: String,
    bazelrc: String,
    mise_toml: String,
    readme: String,
}

impl ProjectScaffold {
    fn for_name(name: &str) -> AtomResult<Self> {
        let bazel_version = pinned_bazel_version();
        let bazelisk_version = mise_tool_version("bazelisk")?;
        let rust_version = mise_tool_version("rust")?;
        let java_version = mise_tool_version("java")?;
        let rules_rust_version = module_dependency_version("rules_rust")?;
        let java_runtime_line = bazelrc_line("build --java_runtime_version=")?;
        let user_bazelrc_import_line = bazelrc_line("try-import %workspace%/user.bazelrc")?;

        Ok(Self {
            module_bazel: render_module_bazel(name, rules_rust_version, rust_version),
            bazelversion: format!("{bazel_version}\n"),
            bazelrc: render_bazelrc(java_runtime_line, user_bazelrc_import_line),
            mise_toml: render_mise_toml(
                bazel_version,
                bazelisk_version,
                rust_version,
                java_version,
            ),
            readme: render_readme(name),
        })
    }
}

fn pinned_bazel_version() -> &'static str {
    ROOT_BAZEL_VERSION.trim()
}

fn mise_tool_version(tool: &str) -> AtomResult<&'static str> {
    quoted_assignment(ROOT_MISE_TOML, tool).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            format!("failed to load scaffold tool version for `{tool}` from mise.toml"),
        )
    })
}

fn module_dependency_version(module_name: &str) -> AtomResult<&'static str> {
    let prefix = format!("bazel_dep(name = \"{module_name}\", version = \"");
    ROOT_MODULE_BAZEL
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|rest| rest.split('"').next())
        })
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                format!("failed to load scaffold module version for `{module_name}`"),
            )
        })
}

fn bazelrc_line(prefix: &str) -> AtomResult<&'static str> {
    ROOT_BAZEL_RC
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(prefix))
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                format!("failed to load scaffold .bazelrc setting `{prefix}`"),
            )
        })
}

fn quoted_assignment<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key} = \"");
    contents.lines().map(str::trim).find_map(|line| {
        line.strip_prefix(&prefix)
            .and_then(|rest| rest.split('"').next())
    })
}

fn render_module_bazel(name: &str, rules_rust_version: &str, rust_version: &str) -> String {
    let framework_version = FRAMEWORK_VERSION;
    let framework_git_remote = FRAMEWORK_GIT_REMOTE;
    let framework_git_branch = FRAMEWORK_GIT_BRANCH;

    format!(
        "module(\n    name = \"{name}\",\n    version = \"0.1.0\",\n)\n\n\
bazel_dep(name = \"atom\", version = \"{framework_version}\")\n\
git_override(\n    module_name = \"atom\",\n    remote = \"{framework_git_remote}\",\n    branch = \"{framework_git_branch}\",\n)\n\n\
bazel_dep(name = \"rules_rust\", version = \"{rules_rust_version}\")\n\n\
rust = use_extension(\"@rules_rust//rust:extensions.bzl\", \"rust\")\n\
rust.toolchain(\n    edition = \"2024\",\n    versions = [\"{rust_version}\"],\n)\n\
use_repo(rust, \"rust_toolchains\")\n\n\
register_toolchains(\"@rust_toolchains//:all\")\n\n\
crate = use_extension(\"@rules_rust//crate_universe:extensions.bzl\", \"crate\")\n\
# Replace this placeholder direct dependency once the first app crate lands.\n\
crate.spec(\n    package = \"camino\",\n    version = \"=1.2.2\",\n)\n\
crate.from_specs(name = \"app_crates\")\n\
use_repo(crate, \"app_crates\")\n",
    )
}

fn render_bazelrc(java_runtime_line: &str, user_bazelrc_import_line: &str) -> String {
    format!(
        "{java_runtime_line}\n\n# Try importing local user config.\n{user_bazelrc_import_line}\n"
    )
}

fn render_mise_toml(
    bazel_version: &str,
    bazelisk_version: &str,
    rust_version: &str,
    java_version: &str,
) -> String {
    format!(
        "[tools]\n\
bazel = \"{bazel_version}\"\n\
bazelisk = \"{bazelisk_version}\"\n\
rust = \"{rust_version}\"\n\
java = \"{java_version}\"\n"
    )
}

fn render_readme(name: &str) -> String {
    format!(
        "# {name}\n\n\
This repository was scaffolded with `atom new`.\n\n\
## Commands\n\n\
```sh\n\
bazelisk build //...\n\
bazelisk run //:atom -- --help\n\
```\n\n\
This scaffold creates the Bazel workspace wrapper only. Add an app crate and `atom_app(...)` target next.\n"
    )
}

const BUILD_BAZEL_TEMPLATE: &str =
    "alias(\n    name = \"atom\",\n    actual = \"@atom//:atom\",\n)\n";

const GITIGNORE_TEMPLATE: &str =
    "/bazel-*\n/.bazel\n/generated/\n/cng-output/\n.DS_Store\nuser.bazelrc\n.mise.local.toml\n";

#[cfg(test)]
mod tests {
    use atom_ffi::AtomErrorCode;

    use super::{
        module_dependency_version, pinned_bazel_version, scaffold_project, validate_project_name,
    };

    #[test]
    fn validate_project_name_rejects_rust_keywords() {
        let error = validate_project_name("crate").expect_err("keywords should fail");

        assert_eq!(error.code, AtomErrorCode::CliUsageError);
        assert!(error.message.contains("reserved in Rust"));
    }

    #[test]
    fn scaffold_project_embeds_framework_pins() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root =
            camino::Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let project = scaffold_project(&root, "my_app").expect("scaffolded project");
        let module_bazel = std::fs::read_to_string(project.join("MODULE.bazel")).expect("module");
        let bazelversion =
            std::fs::read_to_string(project.join(".bazelversion")).expect("bazelversion");

        assert!(module_bazel.contains("bazel_dep(name = \"atom\", version = \"0.1.0\")"));
        assert!(module_bazel.contains(
            "crate = use_extension(\"@rules_rust//crate_universe:extensions.bzl\", \"crate\")"
        ));
        assert!(module_bazel.contains("branch = \"main\""));
        assert!(module_bazel.contains(
            "Replace this placeholder direct dependency once the first app crate lands."
        ));
        assert_eq!(bazelversion, format!("{}\n", pinned_bazel_version()));
        assert!(module_bazel.contains(&format!(
            "bazel_dep(name = \"rules_rust\", version = \"{}\")",
            module_dependency_version("rules_rust").expect("rules_rust version")
        )));
        assert!(module_bazel.contains("crate.from_specs(name = \"app_crates\")"));
        assert!(module_bazel.contains("use_repo(crate, \"app_crates\")"));
        assert!(module_bazel.contains("package = \"camino\""));
    }
}
