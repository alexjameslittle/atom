# Getting Started with Atom

Atom is a Rust-first mobile app framework. This guide takes you from a clean macOS setup to a
running app on an iOS simulator without needing to know Bazel or Rust first.

## Prerequisites

- macOS on Apple Silicon. The published installer currently ships a macOS arm64 binary.
- Xcode 16 or later, including the command line tools.
- Homebrew.
- `mise` for the pinned Bazel, Rust, and Java toolchain:

```sh
brew install mise
```

- Android Studio plus the Android SDK and NDK only if you want Android later.

## Install Atom

```sh
curl -fsSL https://raw.githubusercontent.com/alexjameslittle/atom/main/install.sh | sh
export PATH="$HOME/.atom/bin:$PATH"
atom --version
```

You should see three version lines, for example:

```text
atom 0.1.0
rust 1.92.0
bazel 8.4.2
```

The exact versions may differ.

## Create a project

```sh
atom new my_first_app
cd my_first_app
```

This creates a new app workspace in `./my_first_app`. Depending on the CLI build you installed, Atom
may print a short success message, the absolute path to the new project, or both.

## Install the project toolchain

Run this inside the new project. It installs the exact Bazelisk, Rust, and Java versions pinned by
the scaffolded app:

```sh
mise trust -y mise.toml
mise install
eval "$(mise env)"
```

## Check your environment

Run `atom doctor` from inside the project so it can read the workspace's pinned tool versions:

```sh
atom doctor
```

Look for a summary like:

```text
1 platform ready: ios
```

If you have not set up Android yet, Android warnings are expected and you can ignore them for this
guide.

## Generate native host code

```sh
atom prebuild --target //apps/my_first_app:my_first_app
```

You should see:

```text
generated/ios/my-first-app
generated/android/my-first-app
```

## Run on iOS simulator

```sh
atom run --platform ios --target //apps/my_first_app:my_first_app
```

If Atom finds more than one simulator, it prompts you to choose one. When the command succeeds,
Simulator.app opens and launches `My First App`.

## Run on Android emulator (optional)

After `atom doctor` reports Android ready:

```sh
atom run --platform android --target //apps/my_first_app:my_first_app
```

## What just happened?

- `atom new` created a minimal app workspace with one `atom_app(...)` target in `apps/my_first_app`.
- `mise install` pulled in the toolchain versions pinned by the project.
- `atom prebuild` generated native iOS and Android host trees under `generated/`.
- `atom run` regenerated build files if needed, built the host app, installed it on the selected
  simulator or emulator, and launched it.

## Next steps

- Explore the CLI: `atom --help`
- Read the generated app README: `README.md`
- Read the architecture overview: [architecture.md](architecture.md)
- Browse the rest of the docs: [README.md](README.md)
