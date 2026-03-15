load("@bazel_skylib//rules:write_file.bzl", "write_file")
load("@rules_rust//rust:defs.bzl", "rust_library")

_MODULE_METADATA_SUFFIX = "_atom_module_metadata"

def _package_tail():
    package_name = native.package_name()
    if not package_name:
        return ""
    return package_name.split("/")[-1]

def _target_label(name):
    package_name = native.package_name()
    if package_name:
        return "//{}:{}".format(package_name, name)
    return "//:{}".format(name)

def _absolute_label(label):
    if label.startswith("//") or label.startswith("@"):
        return label
    if label.startswith(":"):
        return _target_label(label[1:])
    fail("Atom labels must be absolute (`//pkg:target`) or package-relative (`:target`), got '{}'".format(label))

def _repo_relative_paths(paths):
    package_name = native.package_name()
    if not package_name:
        return paths
    return ["{}/{}".format(package_name, path) for path in paths]

def _repo_relative_path(path):
    if path == None:
        return None
    package_name = native.package_name()
    if not package_name:
        return path
    return "{}/{}".format(package_name, path)

def _generated_rust_flatbuffers_dep(generated_root, module_id):
    target_prefix = module_id.replace("-", "_")
    package = "{}/flatbuffers/{}".format(generated_root, module_id) if generated_root else "flatbuffers/{}".format(module_id)
    return "//{}:{}_rust_flatbuffers".format(package, target_prefix)

def emit_module_metadata(
        name,
        kind,
        module_id,
        atom_api_level,
        min_atom_version,
        ios_min_deployment_target,
        android_min_sdk,
        crate_root,
        generated_root,
        schema_files,
        depends_on,
        methods,
        permissions,
        plist,
        android_manifest,
        entitlements,
        generated_sources,
        init_priority,
        ios_srcs,
        android_srcs,
        visibility):
    metadata = {
        "kind": kind,
        "target_label": _target_label(name),
        "id": module_id,
        "atom_api_level": atom_api_level,
        "min_atom_version": min_atom_version,
        "ios_min_deployment_target": ios_min_deployment_target,
        "android_min_sdk": android_min_sdk,
        "crate_root": _repo_relative_path(crate_root),
        "generated_root": generated_root,
        "depends_on": [_absolute_label(label) for label in depends_on],
        "schema_files": _repo_relative_paths(schema_files),
        "methods": methods,
        "permissions": permissions,
        "plist": plist,
        "android_manifest": android_manifest,
        "entitlements": entitlements,
        "generated_sources": generated_sources,
        "init_priority": init_priority,
        "ios_srcs": _repo_relative_paths(ios_srcs),
        "android_srcs": _repo_relative_paths(android_srcs),
    }

    write_file(
        name = name + _MODULE_METADATA_SUFFIX,
        out = name + ".atom.module.json",
        content = [json.encode_indent(metadata, indent = "  ")],
        visibility = visibility,
    )

    native.filegroup(
        name = name + "_schema_bundle",
        srcs = schema_files,
        visibility = visibility,
    )

    native.filegroup(
        name = name + "_ios_srcs",
        srcs = ios_srcs,
        visibility = visibility,
    )

    native.filegroup(
        name = name + "_android_srcs",
        srcs = android_srcs,
        visibility = visibility,
    )

def atom_module(
        name,
        module_id = None,
        atom_api_level = 1,
        min_atom_version = None,
        ios_min_deployment_target = None,
        android_min_sdk = None,
        srcs = None,
        crate_root = "src/lib.rs",
        crate_name = None,
        generated_root = "generated",
        schema_files = [],
        depends_on = [],
        methods = [],
        permissions = [],
        plist = {},
        android_manifest = {},
        entitlements = {},
        generated_sources = [],
        init_priority = 0,
        ios_srcs = [],
        android_srcs = [],
        deps = [],
        proc_macro_deps = [],
        visibility = None,
        **kwargs):
    package_tail = _package_tail()
    if package_tail and package_tail != name:
        fail("atom_module name '{}' must match its package directory '{}'".format(name, package_tail))

    target_visibility = visibility if visibility != None else ["//visibility:public"]

    rust_library(
        name = name,
        srcs = srcs if srcs != None else native.glob(["src/**/*.rs"]),
        crate_name = crate_name if crate_name != None else name.replace("-", "_"),
        crate_root = crate_root,
        deps = deps + [_generated_rust_flatbuffers_dep(generated_root, module_id if module_id != None else name)],
        edition = "2024",
        proc_macro_deps = proc_macro_deps,
        visibility = target_visibility,
        **kwargs
    )

    emit_module_metadata(
        name = name,
        kind = "atom_module",
        module_id = module_id if module_id != None else name,
        atom_api_level = atom_api_level,
        min_atom_version = min_atom_version,
        ios_min_deployment_target = ios_min_deployment_target,
        android_min_sdk = android_min_sdk,
        crate_root = crate_root,
        generated_root = generated_root,
        schema_files = schema_files,
        depends_on = depends_on,
        methods = methods,
        permissions = permissions,
        plist = plist,
        android_manifest = android_manifest,
        entitlements = entitlements,
        generated_sources = generated_sources,
        init_priority = init_priority,
        ios_srcs = ios_srcs,
        android_srcs = android_srcs,
        visibility = target_visibility,
    )
