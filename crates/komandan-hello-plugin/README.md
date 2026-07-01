# `komandan-hello-plugin`

Reference **dynamic plugin** for [komandan](https://github.com/hahnavi/komandan)'s
plugin system. Builds as a `cdylib` producing `libkomandan_hello_plugin.so`,
which the komandan host `dlopen`s and dispatches to whenever the user runs:

```
komandan hello [args...]
```

The plugin is intentionally trivial — it prints a greeting and echoes the argv
tail — so it can serve as the canonical example third-party plugin authors
copy. Its real job is to prove the end-to-end plugin loop works:

1. host resolves the `komandan_plugin_v1` entry symbol,
2. calls `register()` and reads the [`PluginDescriptor`],
3. constructs a [`PluginContext`] and calls `run()`,
4. the plugin calls back into the bundled [`LoggerSink`],
5. the plugin returns an [`RString`] greeting the host prints.

It implements the [`Plugin`] trait from the **`komandan-plugin-abi`** interface
crate (the ABI contract between host and plugins; spec:
`docs/PLUGIN_SYSTEM_SPEC.md` in the komandan repository).

## Toolchain

* Rust **nightly**, edition 2024 — matches the komandan workspace and the
  `komandan-plugin-abi` interface crate. Pinned via `rust-toolchain.toml`.
* Depends on `abi_stable = "=0.11.3"` transitively (via the ABI crate).

## Build

```sh
cargo build --release
```

Produces:

```
target/release/libkomandan_hello_plugin.so
```

(debug build → `target/debug/libkomandan_hello_plugin.so`).

This is the shared object the komandan host loads when its plugin loader
scans the plugin directory. Drop it where the host's plugin search path
points (or pass it explicitly via the host's `--plugin-dir` / plugin-path
flag — see komandan's CLI docs).

## CLI invocation served

```
komandan hello foo bar
```

prints something like:

```
Hello from komandan-hello-plugin v0.1.0! (args: foo bar)
```

With no args:

```
komandan hello
```

prints:

```
Hello from komandan-hello-plugin v0.1.0! (args: (no args))
```

## Layout

```
komandan-hello-plugin/
├── Cargo.toml           — package + lint posture (matches komandan workspace)
├── clippy.toml          — too-many-lines-threshold = 500
├── rust-toolchain.toml  — nightly
├── README.md            — this file
└── src/
    └── lib.rs           — HelloPlugin + komandan_plugin_v1 entry symbol + tests
```

## Lint posture

Matches the komandan workspace exactly. Clean under:

```sh
cargo clippy --all-targets -- -D warnings
```

`unsafe_code`, `clippy::pedantic`, `clippy::nursery`, `clippy::enum_glob_use`,
`clippy::unwrap_used`, `clippy::expect_used` are all `deny`. No `unsafe`,
`unwrap`, `expect`, or `panic!` in hand-written code.

## Dependency

Path-pinned to the (unpublished) ABI crate:

```toml
[dependencies]
komandan-plugin-abi = { path = "/home/solo/projects/komandan/crates/komandan-plugin-abi" }
```

There is **no** dependency on the komandan binary or `komandan-core` — only the
ABI interface crate.

[`Plugin`]: https://docs.rs/komandan-plugin-abi/latest/komandan_plugin_abi/trait.Plugin.html
[`PluginDescriptor`]: https://docs.rs/komandan-plugin-abi/latest/komandan_plugin_abi/struct.PluginDescriptor.html
[`PluginContext`]: https://docs.rs/komandan-plugin-abi/latest/komandan_plugin_abi/struct.PluginContext.html
[`LoggerSink`]: https://docs.rs/komandan-plugin-abi/latest/komandan_plugin_abi/trait.LoggerSink.html
[`RString`]: https://docs.rs/abi_stable/latest/abi_stable/std_types/struct.RString.html
