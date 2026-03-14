use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use minijinja::Environment;

fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_keep_trailing_newline(true);
    env.add_template(
        "project/MODULE.bazel",
        include_str!("templates/project/MODULE.bazel.j2"),
    )
    .expect("project/MODULE.bazel template");
    env.add_template(
        "project/.bazelversion",
        include_str!("templates/project/bazelversion.j2"),
    )
    .expect("project/.bazelversion template");
    env.add_template(
        "project/.bazelrc",
        include_str!("templates/project/bazelrc.j2"),
    )
    .expect("project/.bazelrc template");
    env.add_template(
        "project/mise.toml",
        include_str!("templates/project/mise.toml.j2"),
    )
    .expect("project/mise.toml template");
    env.add_template(
        "project/BUILD.bazel",
        include_str!("templates/project/BUILD.bazel.j2"),
    )
    .expect("project/BUILD.bazel template");
    env.add_template(
        "project/README.md",
        include_str!("templates/project/README.md.j2"),
    )
    .expect("project/README.md template");
    env.add_template(
        "project/.gitignore",
        include_str!("templates/project/gitignore.j2"),
    )
    .expect("project/.gitignore template");
    env.add_template(
        "project/platforms/BUILD.bazel",
        include_str!("templates/project/platforms/BUILD.bazel.j2"),
    )
    .expect("project/platforms/BUILD.bazel template");
    env.add_template(
        "project/apps/app/BUILD.bazel",
        include_str!("templates/project/apps/app/BUILD.bazel.j2"),
    )
    .expect("project/apps/app/BUILD.bazel template");
    env.add_template(
        "project/apps/app/src/lib.rs",
        include_str!("templates/project/apps/app/src/lib.rs.j2"),
    )
    .expect("project/apps/app/src/lib.rs template");
    env
}

/// # Errors
///
/// Returns an error if the named template cannot be loaded or rendered.
pub(crate) fn render(name: &str, ctx: minijinja::Value) -> AtomResult<String> {
    let env = env();
    let template = env.get_template(name).map_err(|error| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            format!("failed to load CLI scaffold template {name}: {error}"),
        )
    })?;
    template.render(ctx).map_err(|error| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            format!("failed to render CLI scaffold template {name}: {error}"),
        )
    })
}
