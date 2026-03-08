use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::ModuleRequest;
use camino::Utf8PathBuf;

use crate::{ModuleManifest, ResolvedModule};

pub(crate) type LoadedModule = (ModuleRequest, Utf8PathBuf, ModuleManifest);

pub(crate) fn resolve_loaded_modules(loaded: &[LoadedModule]) -> AtomResult<Vec<ResolvedModule>> {
    let mut by_id = HashMap::new();
    let mut by_target = HashMap::new();

    for (index, (_, _, manifest)) in loaded.iter().enumerate() {
        if by_id.insert(manifest.id.clone(), index).is_some() {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleDuplicateId,
                format!("duplicate module identifier: {}", manifest.id),
                manifest.id.as_str(),
            ));
        }
        by_target.insert(manifest.target_label.clone(), index);
    }

    let mut indegree = vec![0usize; loaded.len()];
    let mut dependents = vec![Vec::new(); loaded.len()];
    let mut layers = vec![0usize; loaded.len()];

    for (index, (_, _, manifest)) in loaded.iter().enumerate() {
        for dependency in &manifest.depends_on {
            let Some(&dependency_index) = by_target.get(dependency) else {
                return Err(AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    format!("unknown dependency target: {dependency}"),
                    format!("modules.{}.depends_on", manifest.id),
                ));
            };
            indegree[index] += 1;
            dependents[dependency_index].push(index);
        }
    }

    let mut ready = VecDeque::new();
    for (index, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.push_back(index);
        }
    }

    let mut resolved_indices = Vec::with_capacity(loaded.len());
    while let Some(index) = ready.pop_front() {
        resolved_indices.push(index);

        let mut children = dependents[index].clone();
        children.sort_unstable();
        for child in children {
            layers[child] = layers[child].max(layers[index] + 1);
            indegree[child] -= 1;
            if indegree[child] == 0 {
                insert_ready(&mut ready, child);
            }
        }
    }

    if resolved_indices.len() != loaded.len() {
        return Err(AtomError::new(
            AtomErrorCode::ModuleDependencyCycle,
            "module dependency cycle detected",
        ));
    }

    let mut init_order = resolved_indices.clone();
    init_order.sort_by_key(|index| {
        (
            layers[*index],
            Reverse(loaded[*index].2.init_priority),
            *index,
        )
    });

    let mut init_positions = HashMap::new();
    for (position, index) in init_order.into_iter().enumerate() {
        init_positions.insert(index, position);
    }

    Ok(resolved_indices
        .into_iter()
        .enumerate()
        .map(|(resolution_index, index)| {
            let (request, metadata_path, manifest) = loaded[index].clone();
            ResolvedModule {
                request,
                metadata_path,
                manifest,
                resolution_index,
                layer: layers[index],
                init_order: init_positions[&index],
            }
        })
        .collect())
}

fn insert_ready(ready: &mut VecDeque<usize>, index: usize) {
    let mut items: Vec<_> = ready.drain(..).collect();
    items.push(index);
    items.sort_unstable();
    ready.extend(items);
}
