load("@bazel_skylib//rules:write_file.bzl", "write_file")
load("@rules_rust//rust:defs.bzl", "rust_library")

_APP_METADATA_SUFFIX = "_atom_app_metadata"

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

def atom_app(
        name,
        app_name = None,
        slug = None,
        srcs = None,
        crate_root = "src/lib.rs",
        crate_name = None,
        deps = [],
        proc_macro_deps = [],
        modules = [],
        generated_root = "generated",
        watch = False,
        ios_enabled = True,
        ios_bundle_id = None,
        ios_deployment_target = None,
        android_enabled = True,
        android_application_id = None,
        android_min_sdk = None,
        android_target_sdk = None,
        visibility = None,
        **kwargs):
    target_visibility = visibility if visibility != None else ["//visibility:public"]
    target_label = _target_label(name)

    rust_library(
        name = name,
        srcs = srcs if srcs != None else native.glob(["src/**/*.rs"]),
        crate_name = crate_name if crate_name != None else name.replace("-", "_"),
        crate_root = crate_root,
        deps = deps,
        edition = "2024",
        proc_macro_deps = proc_macro_deps,
        visibility = target_visibility,
        **kwargs
    )

    metadata = {
        "kind": "atom_app",
        "target_label": target_label,
        "name": app_name if app_name != None else name,
        "slug": slug if slug != None else name.replace("_", "-"),
        "entry_crate_label": target_label,
        "generated_root": generated_root,
        "watch": watch,
        "ios": {
            "enabled": ios_enabled,
            "bundle_id": ios_bundle_id,
            "deployment_target": ios_deployment_target,
        },
        "android": {
            "enabled": android_enabled,
            "application_id": android_application_id,
            "min_sdk": android_min_sdk,
            "target_sdk": android_target_sdk,
        },
        "modules": [_absolute_label(label) for label in modules],
    }

    write_file(
        name = name + _APP_METADATA_SUFFIX,
        out = name + ".atom.app.json",
        content = [json.encode_indent(metadata, indent = "  ")],
        visibility = target_visibility,
    )
