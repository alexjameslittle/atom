load(":atom_module.bzl", "emit_module_metadata")

def atom_native_module(
        name,
        module_id = None,
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
        visibility = None):
    target_visibility = visibility if visibility != None else ["//visibility:public"]

    native.filegroup(
        name = name,
        srcs = ios_srcs + android_srcs + schema_files,
        visibility = target_visibility,
    )

    emit_module_metadata(
        name = name,
        module_id = module_id if module_id != None else name,
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
