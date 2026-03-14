"""Fallback targets when the Android NDK is not installed."""

package(default_visibility = ["//visibility:public"])

exports_files(["target_systems.bzl"])

config_feature_flag(
    name = "true",
    allowed_values = [
        "true",
        "false",
    ],
    default_value = "true",
)

config_setting(
    name = "always_true",
    flag_values = {
        ":true": "true",
    },
)

config_setting(
    name = "always_false",
    flag_values = {
        ":true": "false",
    },
)

alias(
    name = "has_androidndk",
    actual = ":always_false",
)

filegroup(
    name = "files",
    srcs = [":error_message"],
)

filegroup(
    name = "cpufeatures",
    srcs = [":error_message"],
)

filegroup(
    name = "native_app_glue",
    srcs = [":error_message"],
)

genrule(
    name = "invalid_android_ndk_repository_error",
    outs = ["error_message"],
    cmd = """echo \
    android_ndk_repository was used without a valid Android NDK being set. \
    Either the path attribute of android_ndk_repository or the ANDROID_NDK_HOME \
    environment variable must be set. ; \
    exit 1 """,
)
