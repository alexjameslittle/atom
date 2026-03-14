use std::fs;
use std::io::{self, IsTerminal};

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};
use dialoguer::{Input, MultiSelect};
use minijinja::context;

use crate::templates;
use crate::{
    ATOM_APPLE_SUPPORT_VERSION, ATOM_BUILD_BAZEL_VERSION, ATOM_FRAMEWORK_VERSION,
    ATOM_JAVA_RUNTIME_VERSION, ATOM_MISE_BAZELISK_VERSION, ATOM_MISE_JAVA_VERSION,
    ATOM_MISE_RUST_TOOLCHAIN_VERSION, ATOM_PLATFORMS_VERSION, ATOM_RULES_ANDROID_NDK_VERSION,
    ATOM_RULES_ANDROID_VERSION, ATOM_RULES_APPLE_VERSION, ATOM_RULES_JAVA_VERSION,
    ATOM_RULES_KOTLIN_VERSION, ATOM_RULES_RUST_VERSION, ATOM_RULES_SWIFT_VERSION,
};

const FRAMEWORK_GIT_REMOTE: &str = "https://github.com/alexjameslittle/atom.git";
const FRAMEWORK_GIT_BRANCH: &str = "main";
const PLACEHOLDER_CRATE_PACKAGE: &str = "camino";
const PLACEHOLDER_CRATE_VERSION: &str = "=1.2.2";
const DEFAULT_APP_ID_PREFIX: &str = "com.example";
const IOS_PLATFORM_LABEL: &str = "iOS";
const ANDROID_PLATFORM_LABEL: &str = "Android";
const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

const WORKSPACE_SCAFFOLD_FILES: &[(&str, &str)] = &[
    ("MODULE.bazel", "project/MODULE.bazel"),
    (".bazelversion", "project/.bazelversion"),
    (".bazelrc", "project/.bazelrc"),
    ("mise.toml", "project/mise.toml"),
    ("BUILD.bazel", "project/BUILD.bazel"),
    ("README.md", "project/README.md"),
    (".gitignore", "project/.gitignore"),
    ("platforms/BUILD.bazel", "project/platforms/BUILD.bazel"),
];

const APP_SCAFFOLD_FILES: &[(&str, &str)] = &[
    ("BUILD.bazel", "project/apps/app/BUILD.bazel"),
    ("src/lib.rs", "project/apps/app/src/lib.rs"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectPlatforms {
    ios: bool,
    android: bool,
}

impl ProjectPlatforms {
    #[must_use]
    pub(crate) const fn all() -> Self {
        Self {
            ios: true,
            android: true,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn ios_only() -> Self {
        Self {
            ios: true,
            android: false,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn android_only() -> Self {
        Self {
            ios: false,
            android: true,
        }
    }

    fn ensure_selected(self) -> AtomResult<()> {
        if self.ios || self.android {
            return Ok(());
        }

        Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "select at least one platform",
        ))
    }

    #[must_use]
    pub(crate) const fn default_run_platform(self) -> &'static str {
        if self.ios { "ios" } else { "android" }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewProjectConfig {
    pub(crate) name: String,
    display_name: String,
    ios_bundle_id: Option<String>,
    android_application_id: Option<String>,
    platforms: ProjectPlatforms,
}

impl NewProjectConfig {
    /// # Errors
    ///
    /// Returns an error if the project name is invalid.
    pub(crate) fn with_defaults(name: &str) -> AtomResult<Self> {
        let name = normalize_required_value("project name", name)?;
        validate_project_name(&name)?;

        let project_display_name = display_name(&name);
        let app_id = default_app_id(&name);
        Ok(Self {
            name,
            display_name: project_display_name,
            ios_bundle_id: Some(app_id.clone()),
            android_application_id: Some(app_id),
            platforms: ProjectPlatforms::all(),
        })
    }

    fn from_prompt_values(
        name: &str,
        display_name: &str,
        ios_bundle_id: &str,
        android_application_id: &str,
        platforms: ProjectPlatforms,
    ) -> AtomResult<Self> {
        let name = normalize_required_value("project name", name)?;
        validate_project_name(&name)?;
        platforms.ensure_selected()?;

        Ok(Self {
            name,
            display_name: normalize_required_value("display name", display_name)?,
            ios_bundle_id: platforms
                .ios
                .then(|| normalize_required_value("iOS bundle ID", ios_bundle_id))
                .transpose()?,
            android_application_id: platforms
                .android
                .then(|| normalize_required_value("Android package name", android_application_id))
                .transpose()?,
            platforms,
        })
    }

    #[must_use]
    pub(crate) fn default_run_platform(&self) -> &'static str {
        self.platforms.default_run_platform()
    }
}

trait NewProjectPrompter {
    fn prompt_project_name(&mut self) -> AtomResult<String>;
    fn prompt_display_name(&mut self, default: &str) -> AtomResult<Option<String>>;
    fn prompt_ios_bundle_id(&mut self, default: &str) -> AtomResult<Option<String>>;
    fn prompt_android_application_id(&mut self, default: &str) -> AtomResult<Option<String>>;
    fn prompt_platforms(&mut self) -> AtomResult<ProjectPlatforms>;
}

struct DialoguerPrompter;

impl DialoguerPrompter {
    fn prompt_text_with_default(
        prompt: &str,
        default: &str,
        field_name: &str,
    ) -> AtomResult<Option<String>> {
        let value: String = Input::new()
            .with_prompt(prompt)
            .default(default.to_owned())
            .validate_with(|input: &String| validate_required_input(field_name, input))
            .interact_text()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "interactive project creation failed while reading {field_name}: {error}"
                    ),
                )
            })?;
        let value = value.trim().to_owned();
        Ok((value != default).then_some(value))
    }
}

impl NewProjectPrompter for DialoguerPrompter {
    fn prompt_project_name(&mut self) -> AtomResult<String> {
        Input::<String>::new()
            .with_prompt("Project name")
            .validate_with(|input: &String| validate_project_name_input(input))
            .interact_text()
            .map(|value| value.trim().to_owned())
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "interactive project creation failed while reading project name: {error}"
                    ),
                )
            })
    }

    fn prompt_display_name(&mut self, default: &str) -> AtomResult<Option<String>> {
        Self::prompt_text_with_default("Display name", default, "display name")
    }

    fn prompt_ios_bundle_id(&mut self, default: &str) -> AtomResult<Option<String>> {
        Self::prompt_text_with_default("iOS bundle ID", default, "iOS bundle ID")
    }

    fn prompt_android_application_id(&mut self, default: &str) -> AtomResult<Option<String>> {
        Self::prompt_text_with_default("Android package", default, "Android package name")
    }

    fn prompt_platforms(&mut self) -> AtomResult<ProjectPlatforms> {
        let selections = MultiSelect::new()
            .with_prompt("Platforms")
            .items(&[IOS_PLATFORM_LABEL, ANDROID_PLATFORM_LABEL])
            .defaults(&[true, true])
            .interact()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "interactive project creation failed while choosing platforms: {error}"
                    ),
                )
            })?;

        ProjectPlatforms {
            ios: selections.contains(&0),
            android: selections.contains(&1),
        }
        .ensure_selected()?;

        Ok(ProjectPlatforms {
            ios: selections.contains(&0),
            android: selections.contains(&1),
        })
    }
}

/// # Errors
///
/// Returns an error if interactive prompts are unavailable or any prompt value is invalid.
pub(crate) fn prompt_new_project() -> AtomResult<NewProjectConfig> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "project name is required when not attached to an interactive terminal; pass a name or rerun in a TTY",
        ));
    }

    let mut prompts = DialoguerPrompter;
    collect_prompted_project_config(&mut prompts)
}

fn collect_prompted_project_config(
    prompts: &mut impl NewProjectPrompter,
) -> AtomResult<NewProjectConfig> {
    let name = prompts.prompt_project_name()?;
    let default_display_name = display_name(&name);
    let default_ios_bundle_id = default_app_id(&name);
    let default_android_application_id = default_app_id(&name);
    let display_name = prompts
        .prompt_display_name(&default_display_name)?
        .unwrap_or(default_display_name);
    let ios_bundle_id = prompts
        .prompt_ios_bundle_id(&default_ios_bundle_id)?
        .unwrap_or(default_ios_bundle_id);
    let android_application_id = prompts
        .prompt_android_application_id(&default_android_application_id)?
        .unwrap_or(default_android_application_id);
    let platforms = prompts.prompt_platforms()?;

    NewProjectConfig::from_prompt_values(
        &name,
        &display_name,
        &ios_bundle_id,
        &android_application_id,
        platforms,
    )
}

#[must_use]
pub(crate) fn render_success_message(config: &NewProjectConfig) -> String {
    format!(
        "Creating {}...\nDone! Run `cd {} && atom run --platform {}` to get started.\n",
        config.name,
        config.name,
        config.default_run_platform()
    )
}

pub(crate) fn scaffold_project(
    cwd: &Utf8Path,
    config: &NewProjectConfig,
) -> AtomResult<Utf8PathBuf> {
    validate_project_name(&config.name)?;

    let project_root = cwd.join(&config.name);
    if project_root.exists() {
        return Err(AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!(
                "refusing to scaffold `{}` because the target directory already exists",
                config.name
            ),
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
        project_name => config.name.as_str(),
        project_display_name => config.display_name.as_str(),
        project_ios_bundle_id => config.ios_bundle_id.as_deref(),
        project_android_application_id => config.android_application_id.as_deref(),
        ios_enabled => config.platforms.ios,
        android_enabled => config.platforms.android,
        framework_version => ATOM_FRAMEWORK_VERSION,
        framework_git_remote => FRAMEWORK_GIT_REMOTE,
        framework_git_branch => FRAMEWORK_GIT_BRANCH,
        bazel_version => ATOM_BUILD_BAZEL_VERSION,
        bazelisk_version => ATOM_MISE_BAZELISK_VERSION,
        rust_version => ATOM_MISE_RUST_TOOLCHAIN_VERSION,
        java_version => ATOM_MISE_JAVA_VERSION,
        apple_support_version => ATOM_APPLE_SUPPORT_VERSION,
        rules_java_version => ATOM_RULES_JAVA_VERSION,
        rules_kotlin_version => ATOM_RULES_KOTLIN_VERSION,
        rules_android_version => ATOM_RULES_ANDROID_VERSION,
        rules_android_ndk_version => ATOM_RULES_ANDROID_NDK_VERSION,
        platforms_version => ATOM_PLATFORMS_VERSION,
        rules_apple_version => ATOM_RULES_APPLE_VERSION,
        java_runtime_version => ATOM_JAVA_RUNTIME_VERSION,
        rules_rust_version => ATOM_RULES_RUST_VERSION,
        rules_swift_version => ATOM_RULES_SWIFT_VERSION,
        placeholder_crate_package => PLACEHOLDER_CRATE_PACKAGE,
        placeholder_crate_version => PLACEHOLDER_CRATE_VERSION,
    };

    for &(relative_path, template_name) in WORKSPACE_SCAFFOLD_FILES {
        let contents = templates::render(template_name, context.clone())?;
        write_file(&project_root.join(relative_path), &contents)?;
    }

    let app_root = project_root.join("apps").join(&config.name);
    for &(relative_path, template_name) in APP_SCAFFOLD_FILES {
        let contents = templates::render(template_name, context.clone())?;
        write_file(&app_root.join(relative_path), &contents)?;
    }

    Ok(project_root)
}

fn validate_project_name(name: &str) -> AtomResult<()> {
    if let Err(message) = validate_project_name_input(name) {
        return Err(AtomError::new(AtomErrorCode::CliUsageError, message));
    }

    Ok(())
}

fn validate_project_name_input(input: &str) -> Result<(), String> {
    let input = input.trim();
    if !is_valid_project_name(input) {
        return Err(
            "project name must start with a lowercase ASCII letter and contain only lowercase ASCII letters, digits, and underscores"
                .to_owned(),
        );
    }

    if RUST_KEYWORDS.contains(&input) {
        return Err(format!(
            "project name `{input}` is reserved in Rust; choose a different crate name"
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

fn display_name(name: &str) -> String {
    name.split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_ascii_uppercase().to_string();
                    word.extend(characters);
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn default_app_id(name: &str) -> String {
    format!("{DEFAULT_APP_ID_PREFIX}.{name}")
}

fn validate_required_input(field_name: &str, input: &str) -> Result<(), String> {
    if input.trim().is_empty() {
        return Err(format!("{field_name} must not be empty"));
    }

    Ok(())
}

fn normalize_required_value(field_name: &str, value: &str) -> AtomResult<String> {
    let value = value.trim().to_owned();
    validate_required_input(field_name, &value)
        .map(|()| value)
        .map_err(|message| AtomError::new(AtomErrorCode::CliUsageError, message))
}

fn write_file(path: &Utf8Path, contents: &str) -> AtomResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to create scaffold directory: {error}"),
                parent.as_str(),
            )
        })?;
    }

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
    use atom_ffi::{AtomErrorCode, AtomResult};
    use camino::Utf8PathBuf;

    use super::{
        NewProjectConfig, NewProjectPrompter, ProjectPlatforms, collect_prompted_project_config,
        default_app_id, display_name, render_success_message, scaffold_project,
        validate_project_name,
    };

    struct FakePrompter {
        project_name: AtomResult<String>,
        display_name: AtomResult<Option<String>>,
        ios_bundle_id: AtomResult<Option<String>>,
        android_application_id: AtomResult<Option<String>>,
        platforms: AtomResult<ProjectPlatforms>,
    }

    impl NewProjectPrompter for FakePrompter {
        fn prompt_project_name(&mut self) -> AtomResult<String> {
            self.project_name.clone()
        }

        fn prompt_display_name(&mut self, _default: &str) -> AtomResult<Option<String>> {
            self.display_name.clone()
        }

        fn prompt_ios_bundle_id(&mut self, _default: &str) -> AtomResult<Option<String>> {
            self.ios_bundle_id.clone()
        }

        fn prompt_android_application_id(&mut self, _default: &str) -> AtomResult<Option<String>> {
            self.android_application_id.clone()
        }

        fn prompt_platforms(&mut self) -> AtomResult<ProjectPlatforms> {
            self.platforms.clone()
        }
    }

    #[test]
    fn display_name_title_cases_the_project_name() {
        assert_eq!(display_name("my_app"), "My App");
        assert_eq!(display_name("my__app_2"), "My App 2");
    }

    #[test]
    fn default_app_id_uses_the_project_name() {
        assert_eq!(default_app_id("my_app"), "com.example.my_app");
    }

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
        let config = NewProjectConfig::with_defaults("my_app").expect("default config");

        let project_root = scaffold_project(&root, &config).expect("scaffold should succeed");

        assert_eq!(project_root, root.join("my_app"));
        assert_eq!(
            std::fs::read_to_string(project_root.join(".bazelversion")).expect("bazelversion"),
            format!("{}\n", crate::ATOM_BUILD_BAZEL_VERSION)
        );

        let module_bazel =
            std::fs::read_to_string(project_root.join("MODULE.bazel")).expect("module");
        assert!(module_bazel.contains("module_name = \"atom\""));
        assert!(module_bazel.contains("name = \"rules_apple\""));
        assert!(module_bazel.contains("name = \"rules_swift\""));
        assert!(module_bazel.contains("name = \"platforms\""));
        assert!(module_bazel.contains("package = \"camino\""));
        assert!(module_bazel.contains("version = \"=1.2.2\""));
        assert!(module_bazel.contains("extra_target_triples = ["));
        assert!(module_bazel.contains("android_sdk_repository_extension"));

        let app_build =
            std::fs::read_to_string(project_root.join("apps/my_app/BUILD.bazel")).expect("app");
        assert!(app_build.contains("name = \"my_app\""));
        assert!(app_build.contains("crate_name = \"my_app\""));
        assert!(app_build.contains("app_name = \"My App\""));
        assert!(app_build.contains("ios_bundle_id = \"com.example.my_app\""));
        assert!(app_build.contains("android_application_id = \"com.example.my_app\""));
        assert!(app_build.contains("\"@atom//crates/atom-runtime\""));

        let app_lib =
            std::fs::read_to_string(project_root.join("apps/my_app/src/lib.rs")).expect("lib");
        assert!(app_lib.contains("use atom_runtime::RuntimeConfig;"));
        assert!(app_lib.contains("RuntimeConfig::builder().build()"));

        let platforms_build =
            std::fs::read_to_string(project_root.join("platforms/BUILD.bazel")).expect("platforms");
        assert!(platforms_build.contains("name = \"arm64-v8a\""));
        assert!(platforms_build.contains("@platforms//os:android"));
        assert!(platforms_build.contains("@platforms//cpu:arm64"));
    }

    #[test]
    fn scaffold_project_omits_android_config_when_only_ios_is_selected() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let config = NewProjectConfig {
            name: "weather".to_owned(),
            display_name: "Weather".to_owned(),
            ios_bundle_id: Some("com.example.weather".to_owned()),
            android_application_id: None,
            platforms: ProjectPlatforms::ios_only(),
        };

        let project_root = scaffold_project(&root, &config).expect("scaffold should succeed");
        let app_build =
            std::fs::read_to_string(project_root.join("apps/weather/BUILD.bazel")).expect("app");

        assert!(app_build.contains("ios_bundle_id = \"com.example.weather\""));
        assert!(app_build.contains("ios_deployment_target = \"18.0\""));
        assert!(app_build.contains("android_enabled = False"));
        assert!(!app_build.contains("android_application_id"));
        assert!(!app_build.contains("android_min_sdk"));
        assert!(!app_build.contains("android_target_sdk"));
    }

    #[test]
    fn prompted_project_config_uses_defaults_for_empty_answers() {
        let mut prompts = FakePrompter {
            project_name: Ok("my_weather_app".to_owned()),
            display_name: Ok(None),
            ios_bundle_id: Ok(None),
            android_application_id: Ok(None),
            platforms: Ok(ProjectPlatforms::all()),
        };

        let config = collect_prompted_project_config(&mut prompts)
            .expect("prompt collection should succeed");

        assert_eq!(config.name, "my_weather_app");
        assert_eq!(config.display_name, "My Weather App");
        assert_eq!(
            config.ios_bundle_id.as_deref(),
            Some("com.example.my_weather_app")
        );
        assert_eq!(
            config.android_application_id.as_deref(),
            Some("com.example.my_weather_app")
        );
    }

    #[test]
    fn prompted_project_config_omits_unselected_platform_ids() {
        let mut prompts = FakePrompter {
            project_name: Ok("weather".to_owned()),
            display_name: Ok(Some("Weather".to_owned())),
            ios_bundle_id: Ok(Some("com.example.weather".to_owned())),
            android_application_id: Ok(Some("com.example.weather".to_owned())),
            platforms: Ok(ProjectPlatforms::ios_only()),
        };

        let config = collect_prompted_project_config(&mut prompts)
            .expect("prompt collection should succeed");

        assert_eq!(config.ios_bundle_id.as_deref(), Some("com.example.weather"));
        assert_eq!(config.android_application_id, None);
        assert_eq!(config.default_run_platform(), "ios");
    }

    #[test]
    fn prompted_project_config_requires_at_least_one_platform() {
        let mut prompts = FakePrompter {
            project_name: Ok("weather".to_owned()),
            display_name: Ok(None),
            ios_bundle_id: Ok(None),
            android_application_id: Ok(None),
            platforms: Ok(ProjectPlatforms {
                ios: false,
                android: false,
            }),
        };

        let error = collect_prompted_project_config(&mut prompts)
            .expect_err("missing platforms should fail");

        assert_eq!(error.code, AtomErrorCode::CliUsageError);
        assert!(error.message.contains("select at least one platform"));
    }

    #[test]
    fn render_success_message_prefers_the_first_selected_platform() {
        let config = NewProjectConfig::with_defaults("my_app").expect("default config");

        assert_eq!(
            render_success_message(&config),
            "Creating my_app...\nDone! Run `cd my_app && atom run --platform ios` to get started.\n"
        );
    }

    #[test]
    fn render_success_message_uses_android_when_its_the_only_platform() {
        let config = NewProjectConfig {
            name: "my_app".to_owned(),
            display_name: "My App".to_owned(),
            ios_bundle_id: None,
            android_application_id: Some("com.example.my_app".to_owned()),
            platforms: ProjectPlatforms::android_only(),
        };

        assert_eq!(
            render_success_message(&config),
            "Creating my_app...\nDone! Run `cd my_app && atom run --platform android` to get started.\n"
        );
    }
}
