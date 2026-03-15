use std::collections::{BTreeMap, BTreeSet};
use std::{fs, io};

use atom_backends::{ContributedFile, FileSource, GenerationBackendRegistry, GenerationPlan};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

/// # Errors
///
/// Returns an error if any generated file or directory cannot be written.
///
/// # Panics
///
/// Panics if platform configs are missing when the corresponding platform plan exists, or if
/// schema files lack the expected generated prefix.
pub fn emit_host_tree(
    repo_root: &Utf8Path,
    plan: &GenerationPlan,
    registry: &GenerationBackendRegistry,
) -> AtomResult<Vec<Utf8PathBuf>> {
    reset_flatbuffer_packages(repo_root, plan)?;
    copy_contributed_files(repo_root, &plan.contributed_files)?;
    for backend in registry.iter() {
        backend.emit_host_tree(repo_root, plan)?;
    }
    Ok(generated_roots(plan, registry))
}

fn reset_flatbuffer_packages(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    let mut expected_by_root: BTreeMap<Utf8PathBuf, BTreeSet<String>> = BTreeMap::new();
    for module in &plan.modules {
        expected_by_root
            .entry(module.manifest.generated_root.clone())
            .or_default()
            .insert(module.manifest.id.clone());
    }

    for (generated_root, expected_modules) in expected_by_root {
        let flatbuffers_root = repo_root.join(&generated_root).join("flatbuffers");
        remove_stale_flatbuffer_packages(&flatbuffers_root, &expected_modules)?;
        for module_id in expected_modules {
            remove_existing_path(&flatbuffers_root.join(module_id))?;
        }
    }
    Ok(())
}

fn remove_stale_flatbuffer_packages(
    flatbuffers_root: &Utf8Path,
    expected_modules: &BTreeSet<String>,
) -> AtomResult<()> {
    let entries = match fs::read_dir(flatbuffers_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to read generated flatbuffers directory: {error}"),
                flatbuffers_root.as_str(),
            ));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to read generated flatbuffers directory entry: {error}"),
                flatbuffers_root.as_str(),
            )
        })?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                "generated flatbuffers directory entry path must be valid UTF-8",
                flatbuffers_root.as_str(),
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to read generated flatbuffers entry type: {error}"),
                path.as_str(),
            )
        })?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() && !expected_modules.contains(&file_name) {
            remove_existing_path(&path)?;
        }
    }
    Ok(())
}

fn copy_contributed_files(repo_root: &Utf8Path, files: &[ContributedFile]) -> AtomResult<()> {
    for file in files {
        let destination = repo_root.join(&file.output);
        match &file.source {
            FileSource::Copy(source) => copy_path(&repo_root.join(source), &destination)?,
            FileSource::Content(contents) => write_file(&destination, contents)?,
        }
    }
    Ok(())
}

fn generated_roots(
    plan: &GenerationPlan,
    registry: &GenerationBackendRegistry,
) -> Vec<Utf8PathBuf> {
    registry
        .iter()
        .filter_map(|backend| backend.generated_root(plan))
        .collect()
}

/// # Errors
///
/// Returns an error if the destination parent directory cannot be created or the file cannot be
/// written.
pub fn write_file(path: &Utf8Path, contents: &str) -> AtomResult<()> {
    write_parent_dir(path)?;
    fs::write(path, contents).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to write generated file: {error}"),
            path.as_str(),
        )
    })
}

fn write_parent_dir(path: &Utf8Path) -> AtomResult<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to create parent directory: {error}"),
            parent.as_str(),
        )
    })
}

fn copy_path(source: &Utf8Path, destination: &Utf8Path) -> AtomResult<()> {
    let metadata = fs::metadata(source).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to stat contributed source: {error}"),
            source.as_str(),
        )
    })?;

    if metadata.is_dir() {
        remove_existing_path(destination)?;
        fs::create_dir_all(destination).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to create contributed directory: {error}"),
                destination.as_str(),
            )
        })?;
        for entry in fs::read_dir(source).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to read contributed directory: {error}"),
                source.as_str(),
            )
        })? {
            let entry = entry.map_err(|error| {
                AtomError::with_path(
                    AtomErrorCode::CngWriteError,
                    format!("failed to read contributed directory entry: {error}"),
                    source.as_str(),
                )
            })?;
            let entry_path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
                AtomError::with_path(
                    AtomErrorCode::CngWriteError,
                    "contributed directory entry path must be valid UTF-8",
                    source.as_str(),
                )
            })?;
            let destination_entry = destination.join(entry.file_name().to_string_lossy().as_ref());
            copy_path(&entry_path, &destination_entry)?;
        }
        return Ok(());
    }

    remove_existing_path(destination)?;
    write_parent_dir(destination)?;
    fs::copy(source, destination).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to copy contributed file: {error}"),
            destination.as_str(),
        )
    })?;
    Ok(())
}

fn remove_existing_path(path: &Utf8Path) -> AtomResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() {
                fs::remove_dir_all(path).map_err(|error| {
                    AtomError::with_path(
                        AtomErrorCode::CngWriteError,
                        format!("failed to remove stale contributed directory: {error}"),
                        path.as_str(),
                    )
                })?;
            } else {
                fs::remove_file(path).map_err(|error| {
                    AtomError::with_path(
                        AtomErrorCode::CngWriteError,
                        format!("failed to remove stale contributed file: {error}"),
                        path.as_str(),
                    )
                })?;
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to stat contributed destination: {error}"),
            path.as_str(),
        )),
    }
}
