_PLUGIN_ID = "app_icon"
_PLUGIN_TARGET = str(Label("//crates/atom-cng-app-icon:atom-cng-app-icon"))

def _repo_relative_path(path):
    if path == None:
        return None
    if path.startswith("/"):
        fail("atom_app_icon paths must be repo-relative, got '{}'".format(path))
    package_name = native.package_name()
    if not package_name:
        return path
    return "{}/{}".format(package_name, path)

def atom_app_icon(
        ios = None,
        android = None,
        min_atom_version = None,
        ios_min_deployment_target = None,
        android_min_sdk = None):
    config = {}
    if ios != None:
        config["ios"] = _repo_relative_path(ios)
    if android != None:
        config["android"] = _repo_relative_path(android)

    plugin = {
        "id": _PLUGIN_ID,
        "target_label": _PLUGIN_TARGET,
        "atom_api_level": 1,
        "config": config,
    }
    if min_atom_version != None:
        plugin["min_atom_version"] = min_atom_version
    if ios != None:
        plugin["ios_min_deployment_target"] = ios_min_deployment_target or "18.0"
    elif ios_min_deployment_target != None:
        plugin["ios_min_deployment_target"] = ios_min_deployment_target
    if android_min_sdk != None:
        plugin["android_min_sdk"] = android_min_sdk
    return plugin
