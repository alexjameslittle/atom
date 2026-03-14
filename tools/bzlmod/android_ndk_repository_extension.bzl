"""Bzlmod extension that lets non-Android Bazel targets build without a configured NDK."""

# The happy-path implementation mirrors rules_android_ndk so Android builds still use the upstream
# repository layout and toolchains. The only behavior change is an empty fallback repository when
# ANDROID_NDK_HOME is unset, matching rules_android's SDK fallback semantics.

def _optional_android_ndk_repository_impl(ctx):
    ndk_path = ctx.attr.path or ctx.os.environ.get("ANDROID_NDK_HOME", None)
    if not ndk_path:
        ctx.template(
            "BUILD.bazel",
            ctx.attr._template_empty_ndk_root,
            {},
            executable = False,
        )
        ctx.template(
            "target_systems.bzl",
            ctx.attr._template_target_systems,
            {},
            executable = False,
        )
        return

    if ndk_path.startswith("$WORKSPACE_ROOT"):
        ndk_path = str(ctx.workspace_root) + ndk_path.removeprefix("$WORKSPACE_ROOT")

    is_windows = False
    executable_extension = ""
    if ctx.os.name == "linux":
        clang_directory = "toolchains/llvm/prebuilt/linux-x86_64"
    elif ctx.os.name == "mac os x":
        # darwin-x86_64 contains universal binaries that also work on arm64 hosts.
        clang_directory = "toolchains/llvm/prebuilt/darwin-x86_64"
    elif ctx.os.name.startswith("windows"):
        clang_directory = "toolchains/llvm/prebuilt/windows-x86_64"
        is_windows = True
        executable_extension = ".exe"
    else:
        fail("Unsupported operating system: " + ctx.os.name)

    sysroot_directory = "%s/sysroot" % clang_directory
    _create_symlinks(ctx, ndk_path, clang_directory, sysroot_directory)

    api_level = ctx.attr.api_level or 31

    result = ctx.execute([clang_directory + "/bin/clang", "--print-resource-dir"])
    if result.return_code != 0:
        fail("Failed to execute clang: %s" % result.stderr)
    stdout = result.stdout.strip()
    if is_windows:
        stdout = stdout.replace("\\", "/")
    clang_resource_directory = stdout.split(clang_directory)[1].strip("/")

    repository_name = ctx.attr._build.workspace_name

    ctx.template(
        "BUILD.bazel",
        ctx.attr._template_ndk_root,
        {
            "{clang_directory}": clang_directory,
        },
        executable = False,
    )

    ctx.template(
        "target_systems.bzl",
        ctx.attr._template_target_systems,
        {},
        executable = False,
    )

    ctx.template(
        "%s/BUILD.bazel" % clang_directory,
        ctx.attr._template_ndk_clang,
        {
            "{repository_name}": repository_name,
            "{api_level}": str(api_level),
            "{clang_resource_directory}": clang_resource_directory,
            "{sysroot_directory}": sysroot_directory,
            "{executable_extension}": executable_extension,
        },
        executable = False,
    )

    ctx.template(
        "%s/BUILD.bazel" % sysroot_directory,
        ctx.attr._template_ndk_sysroot,
        {
            "{api_level}": str(api_level),
        },
        executable = False,
    )

def _create_symlinks(ctx, ndk_path, clang_directory, sysroot_directory):
    if not ndk_path.endswith("/"):
        ndk_path = ndk_path + "/"

    for path in ctx.path(ndk_path + clang_directory).readdir():
        repo_relative_path = str(path).replace(ndk_path, "")
        if repo_relative_path != sysroot_directory:
            ctx.symlink(path, repo_relative_path)

    for path in ctx.path(ndk_path + sysroot_directory).readdir():
        repo_relative_path = str(path).replace(ndk_path, "")
        ctx.symlink(path, repo_relative_path)

    ctx.symlink(ndk_path + "sources", "sources")
    ctx.symlink(ndk_path + "sources", "ndk/sources")

optional_android_ndk_repository = repository_rule(
    attrs = {
        "path": attr.string(),
        "api_level": attr.int(),
        "_build": attr.label(default = "@rules_android_ndk//:BUILD", allow_single_file = True),
        "_template_empty_ndk_root": attr.label(
            default = "//tools/bzlmod:BUILD.ndk_root.empty.tpl",
            allow_single_file = True,
        ),
        "_template_ndk_root": attr.label(
            default = "@rules_android_ndk//:BUILD.ndk_root.tpl",
            allow_single_file = True,
        ),
        "_template_target_systems": attr.label(
            default = "@rules_android_ndk//:target_systems.bzl.tpl",
            allow_single_file = True,
        ),
        "_template_ndk_clang": attr.label(
            default = "@rules_android_ndk//:BUILD.ndk_clang.tpl",
            allow_single_file = True,
        ),
        "_template_ndk_sysroot": attr.label(
            default = "@rules_android_ndk//:BUILD.ndk_sysroot.tpl",
            allow_single_file = True,
        ),
    },
    local = True,
    implementation = _optional_android_ndk_repository_impl,
)

def _optional_android_ndk_repository_extension_impl(module_ctx):
    root_modules = [m for m in module_ctx.modules if m.is_root and m.tags.configure]
    if len(root_modules) > 1:
        fail(
            "Expected at most one root module, found {}".format(
                ", ".join([module.name for module in root_modules]),
            ),
        )

    if root_modules:
        module = root_modules[0]
    else:
        module = module_ctx.modules[0]

    kwargs = {}
    if module.tags.configure:
        kwargs["api_level"] = module.tags.configure[0].api_level
        kwargs["path"] = module.tags.configure[0].path

    optional_android_ndk_repository(
        name = "androidndk",
        **kwargs
    )

optional_android_ndk_repository_extension = module_extension(
    implementation = _optional_android_ndk_repository_extension_impl,
    tag_classes = {
        "configure": tag_class(attrs = {
            "path": attr.string(),
            "api_level": attr.int(),
        }),
    },
)
