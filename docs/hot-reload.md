# GPUI hot reload

Run `npm run dev:hot` to develop with GPUI's Subsecond hot-patch integration. The launcher downloads the pinned Dioxus CLI into `target/dev-tools/` on first use and checks its SHA-256 before extracting it. `npm run dev:hot:setup` installs the tool without starting the app. Windows needs the MSVC Rust toolchain and the bundled Windows `tar`; macOS and Linux need `tar` as well as the normal GPUI build dependencies.

```sh
npm run dev:hot
npm run dev:hot -- --showcase
npm run dev:hot -- --workspace "<copy of a workspace>"
```

The default data directory is `target/hot-reload/data`, with a separate workspace under `target/hot-reload/workspace`. These settings persist between development sessions. Pass `--data-dir <folder>` to choose another development data directory. Use a workspace copy for development; edits still save to the workspace selected in that app.

The build cache is isolated in `target/hot-reload/build`, so the normal app and tests can keep using `target/debug`. The first build compiles the framework; later edits rebuild the changed code. `npm start` and ordinary Cargo builds keep their existing behavior. `PRESENTER_ORBIT_DX` can point to a separately installed CLI of the pinned version.

The optional `orbit-app/dev-hot-reload` feature enables the existing `gpui-pre/hot-reload` integration. GPUI applies patches between foreground updates, refreshes its windows, and keeps existing entities. Change a `Render` or `RenderOnce` implementation, save it, and watch the terminal for `Hot-patching`. The launcher passes app arguments as JSON through `PRESENTER_ORBIT_HOT_ARGS` because dx 0.7 splits even quoted `--args` on whitespace. That bridge is compiled only into debug builds with the feature enabled; helper and output-process argument parsing is unchanged.

On Windows x64, this feature also includes the stack probe in the app's own object files. The pinned CLI's external-function trampoline uses RAX for the destination address, but `__chkstk` receives the allocation size in that register. Routing the probe through that trampoline causes a stack overflow after patching. Keeping the probe local preserves its register contract; normal builds still use the CRT implementation. Remove this workaround when upgrading to a CLI that preserves RAX in its Windows trampolines. See the [CLI trampoline implementation](https://github.com/DioxusLabs/dioxus/blob/v0.7.10/packages/cli/src/build/patch.rs) and [LLVM's stack-probe ABI notes](https://github.com/llvm/llvm-project/blob/main/compiler-rt/lib/builtins/x86_64/chkstk.S).

Restart the development command after changing struct fields, enum layouts, initialization, or dependencies. Startup-created services and callbacks are not automatically recreated. Custom `Element` layout/paint implementations and the separate native output process are outside GPUI's normal render hot-patch boundary. Embedded assets, translations, or cached theme data can also require a restart. This is a development tool; hot reload is not enabled in normal builds and GPUI disables its runtime in release builds.

Validated on Windows x64 with consecutive changes in both `orbit-app` and `orbit-ui`: the same window and processes remained alive, and a slider value set before patching was preserved. Patches took approximately 15–23 seconds on the test machine; Rust still needs to compile the changed code. macOS and Linux use the existing upstream integration but have not been tested here.

Upstream references: [GPUI integration](https://github.com/zed-industries/zed/pull/65738), [Subsecond limitations](https://docs.rs/subsecond/0.7.10/subsecond/).
