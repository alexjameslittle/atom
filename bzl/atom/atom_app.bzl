load("@bazel_skylib//rules:write_file.bzl", "write_file")
load("@rules_rust//rust:defs.bzl", "rust_library")

_APP_METADATA_SUFFIX = "_atom_app_metadata"
_CONFIG_PLUGIN_REQUIRED_KEYS = (
    "id",
    "target_label",
    "atom_api_level",
    "config",
)
_CONFIG_PLUGIN_OPTIONAL_KEYS = (
    "min_atom_version",
    "ios_min_deployment_target",
    "android_min_sdk",
)

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

def _normalize_config_plugin(plugin):
    if type(plugin) != "dict":
        fail("atom_app config_plugins entries must be dicts, got {}".format(type(plugin)))

    allowed_keys = _CONFIG_PLUGIN_REQUIRED_KEYS + _CONFIG_PLUGIN_OPTIONAL_KEYS
    for key in plugin.keys():
        if key not in allowed_keys:
            fail("atom_app config_plugins entry contains unknown key '{}'".format(key))

    missing = [key for key in _CONFIG_PLUGIN_REQUIRED_KEYS if key not in plugin]
    if missing:
        fail("atom_app config_plugins entry is missing required keys: {}".format(", ".join(missing)))

    if type(plugin["id"]) != "string" or not plugin["id"]:
        fail("atom_app config_plugins entries must declare a non-empty string id")
    if type(plugin["atom_api_level"]) != "int":
        fail("atom_app config_plugins '{}' must declare integer atom_api_level".format(plugin["id"]))
    if type(plugin["config"]) != "dict":
        fail("atom_app config_plugins '{}' must declare config as a dict".format(plugin["id"]))

    normalized = {
        "id": plugin["id"],
        "target_label": _absolute_label(str(plugin["target_label"])),
        "atom_api_level": plugin["atom_api_level"],
        "config": plugin["config"],
    }

    for key in _CONFIG_PLUGIN_OPTIONAL_KEYS:
        if plugin.get(key) != None:
            normalized[key] = plugin[key]

    return normalized

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
        automation_fixture = False,
        config_plugins = [],
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
        "entry_crate_name": crate_name if crate_name != None else name.replace("-", "_"),
        "generated_root": generated_root,
        "watch": watch,
        "automation_fixture": automation_fixture,
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
        "config_plugins": [_normalize_config_plugin(plugin) for plugin in config_plugins],
    }

    write_file(
        name = name + _APP_METADATA_SUFFIX,
        out = name + ".atom.app.json",
        content = [json.encode_indent(metadata, indent = "  ")],
        visibility = target_visibility,
    )
