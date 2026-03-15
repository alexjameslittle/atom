use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_modules::{ModuleKind, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::json;

use crate::rust_source::{parse_rust_module, render_flatbuffer_schema};
use crate::templates;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFlatbufferPackage {
    pub module_id: String,
    pub package_root: Utf8PathBuf,
    pub build_file: Utf8PathBuf,
    pub rust_wrapper: Utf8PathBuf,
    pub generated_schema: Option<GeneratedSchemaFile>,
    pub source_schemas: Vec<SchemaSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSchemaFile {
    pub path: Utf8PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSource {
    pub label: String,
    pub stem: String,
    pub kotlin_out: Utf8PathBuf,
}

impl ModuleFlatbufferPackage {
    /// # Errors
    ///
    /// Returns an error if the generated BUILD template cannot be rendered.
    pub fn build_contents(&self) -> AtomResult<String> {
        let target_prefix = self.module_id.replace('-', "_");
        let rust_rule_name = format!("{target_prefix}_rust_flatbuffers_srcs");
        let swift_rule_name = format!("{target_prefix}_swift_flatbuffers_srcs");
        let kotlin_rule_name = format!("{target_prefix}_kotlin_flatbuffers_srcs");
        templates::render(
            "flatbuffers/BUILD.bazel",
            minijinja::context! {
                kotlin_rule_name,
                rust_outs => flatbuffer_outputs(&self.source_schemas, "rs"),
                rust_rule_name,
                source_schemas => source_schemas_context(&self.source_schemas),
                swift_module_name => format!("{}Flatbuffers", upper_camel_case(&target_prefix)),
                swift_outs => flatbuffer_outputs(&self.source_schemas, "swift"),
                swift_rule_name,
                target_prefix,
            },
        )
    }

    /// # Errors
    ///
    /// Returns an error if the generated Rust wrapper template cannot be rendered.
    pub fn rust_wrapper_contents(&self) -> AtomResult<String> {
        templates::render(
            "flatbuffers/lib.rs",
            minijinja::context! {
                source_schemas => source_schemas_context(&self.source_schemas),
            },
        )
    }
}

/// # Errors
///
/// Returns an error if the module's generated schema/build package cannot be planned from the
/// available metadata and source inputs.
pub fn plan_module_flatbuffers(
    repo_root: &Utf8Path,
    module: &ResolvedModule,
) -> AtomResult<ModuleFlatbufferPackage> {
    let package_root = module
        .manifest
        .generated_root
        .join("flatbuffers")
        .join(&module.manifest.id);
    let build_file = package_root.join("BUILD.bazel");
    let rust_wrapper = package_root.join("lib.rs");

    match module.manifest.kind {
        ModuleKind::Rust => {
            let crate_root = module.manifest.crate_root.as_ref().ok_or_else(|| {
                AtomError::with_path(
                    AtomErrorCode::CngTemplateError,
                    "rust module metadata must include crate_root",
                    module.metadata_path.as_str(),
                )
            })?;
            let parsed = parse_rust_module(repo_root, crate_root)?;
            if parsed.is_empty() {
                return Err(AtomError::with_path(
                    AtomErrorCode::CngTemplateError,
                    "rust module must declare at least one #[atom_record], #[atom_export], or #[atom_import] item",
                    crate_root.as_str(),
                ));
            }
            let schema_name = module.manifest.id.replace('-', "_");
            let schema_path = package_root.join(format!("{schema_name}.fbs"));
            let schema_contents = render_flatbuffer_schema(&module.manifest.id, &parsed)?;
            Ok(ModuleFlatbufferPackage {
                module_id: module.manifest.id.clone(),
                package_root,
                build_file,
                rust_wrapper,
                generated_schema: Some(GeneratedSchemaFile {
                    path: schema_path,
                    contents: schema_contents,
                }),
                source_schemas: vec![SchemaSource {
                    label: format!("{schema_name}.fbs"),
                    stem: schema_name.clone(),
                    kotlin_out: kotlin_onefile_output(
                        &["atom".to_owned(), module.manifest.id.replace('-', "_")],
                        &schema_name,
                    ),
                }],
            })
        }
        ModuleKind::Native => {
            let source_schemas = module
                .manifest
                .schema_files
                .iter()
                .map(|schema_path| plan_native_schema_source(repo_root, module, schema_path))
                .collect::<AtomResult<Vec<_>>>()?;
            Ok(ModuleFlatbufferPackage {
                module_id: module.manifest.id.clone(),
                package_root,
                build_file,
                rust_wrapper,
                generated_schema: None,
                source_schemas,
            })
        }
    }
}

fn source_schemas_context(source_schemas: &[SchemaSource]) -> Vec<serde_json::Value> {
    source_schemas
        .iter()
        .map(|schema| {
            json!({
                "kotlin_out": schema.kotlin_out.as_str(),
                "label": schema.label,
                "stem": schema.stem,
            })
        })
        .collect()
}

fn plan_native_schema_source(
    repo_root: &Utf8Path,
    module: &ResolvedModule,
    schema_path: &Utf8Path,
) -> AtomResult<SchemaSource> {
    let namespace = parse_flatbuffer_namespace(&repo_root.join(schema_path))?;
    let label = bazel_file_label(&module.manifest.target_label, schema_path)?;
    let stem = schema_path
        .file_stem()
        .ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::CngTemplateError,
                "native module schema file must have a file stem",
                schema_path.as_str(),
            )
        })?
        .replace('-', "_");

    Ok(SchemaSource {
        label,
        stem: stem.clone(),
        kotlin_out: kotlin_onefile_output(&namespace, &stem),
    })
}

fn flatbuffer_outputs(sources: &[SchemaSource], extension: &str) -> Vec<String> {
    sources
        .iter()
        .map(|schema| format!("{}_generated.{extension}", schema.stem))
        .collect()
}

fn bazel_file_label(target_label: &str, repo_relative_path: &Utf8Path) -> AtomResult<String> {
    let package = target_label
        .strip_prefix("//")
        .and_then(|value| value.split_once(':').map(|(package, _)| package))
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::CngTemplateError,
                format!("unsupported module target label for schema generation: {target_label}"),
            )
        })?;
    let prefix = if package.is_empty() {
        String::new()
    } else {
        format!("{package}/")
    };
    let relative = repo_relative_path
        .as_str()
        .strip_prefix(&prefix)
        .ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::CngTemplateError,
                "module schema file must live under the module's Bazel package",
                repo_relative_path.as_str(),
            )
        })?;
    Ok(format!("//{package}:{relative}"))
}

fn parse_flatbuffer_namespace(path: &Utf8Path) -> AtomResult<Vec<String>> {
    let contents = fs::read_to_string(path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngTemplateError,
            format!("failed to read FlatBuffers schema: {error}"),
            path.as_str(),
        )
    })?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(namespace) = trimmed
            .strip_prefix("namespace ")
            .and_then(|value| value.strip_suffix(';'))
        {
            return Ok(namespace
                .split('.')
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned)
                .collect());
        }
    }
    Ok(Vec::new())
}

fn kotlin_onefile_output(namespace: &[String], stem: &str) -> Utf8PathBuf {
    let mut output = Utf8PathBuf::new();
    for segment in namespace {
        output.push(segment);
    }
    output.push(format!("{stem}.kt"));
    output
}

fn upper_camel_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let mut output = String::new();
            if let Some(first) = chars.next() {
                output.push(first.to_ascii_uppercase());
            }
            output.extend(chars);
            output
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_modules::testing::{fixture_resolved_module, fixture_schema_module};
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::plan_module_flatbuffers;

    #[test]
    fn plans_rust_module_generated_schema_and_build_package() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("modules/fixture/src")).expect("src dir");
        fs::write(
            root.join("modules/fixture/src/lib.rs"),
            r#"
#[atom_macros::atom_record]
struct DeviceInfo {
    model: String,
}
"#,
        )
        .expect("source");

        let module = fixture_resolved_module(&root);
        let package = plan_module_flatbuffers(&root, &module).expect("package");

        assert_eq!(
            package
                .generated_schema
                .as_ref()
                .map(|schema| schema.path.clone()),
            Some(Utf8PathBuf::from(
                "generated/flatbuffers/fixture_module/fixture_module.fbs"
            ))
        );
        assert!(
            package
                .build_contents()
                .expect("build")
                .contains("@flatbuffers//swift")
        );
        assert!(
            package
                .build_contents()
                .expect("build")
                .contains("kt_jvm_library(")
        );
        assert!(
            package
                .rust_wrapper_contents()
                .expect("wrapper")
                .contains("fixture_module_generated")
        );
    }

    #[test]
    fn plans_native_module_against_existing_schema_labels() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("modules/schema")).expect("schema dir");
        fs::write(
            root.join("modules/schema/fixture.fbs"),
            "namespace atom.native_fixture;\n",
        )
        .expect("schema");

        let module = fixture_schema_module(&root, "modules/schema/fixture.fbs");
        let package = plan_module_flatbuffers(&root, &module).expect("package");

        assert!(package.generated_schema.is_none());
        assert_eq!(
            package.source_schemas[0].label,
            "//modules/schema:fixture.fbs"
        );
        assert_eq!(
            package.source_schemas[0].kotlin_out,
            Utf8PathBuf::from("atom/native_fixture/fixture.kt")
        );
    }
}
