"Linter aspects for the Atom repository."

load("@aspect_rules_lint//lint:clippy.bzl", "lint_clippy_aspect")
load("@aspect_rules_lint//lint:lint_test.bzl", "lint_test")

clippy = lint_clippy_aspect(
    config = Label("//:clippy.toml"),
    clippy_flags = [
        "-Dwarnings",
        "-Wclippy::pedantic",
        "-Fclippy::blanket_clippy_restriction_lints",
        "-Dclippy::allow_attributes",
        "-Dclippy::allow_attributes_without_reason",
    ],
)

clippy_test = lint_test(aspect = clippy)
