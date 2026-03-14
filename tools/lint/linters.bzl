"Linter aspects for the Atom repository."

load("@aspect_rules_lint//lint:clippy.bzl", "lint_clippy_aspect")
load("@aspect_rules_lint//lint:lint_test.bzl", "lint_test")
load("@aspect_rules_lint//lint:ruff.bzl", "lint_ruff_aspect")

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

ruff = lint_ruff_aspect(
    binary = Label("@aspect_rules_lint//lint:ruff_bin"),
    configs = [Label("//:.ruff.toml")],
)

ruff_test = lint_test(aspect = ruff)
