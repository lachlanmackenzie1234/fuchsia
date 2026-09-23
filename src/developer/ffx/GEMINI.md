# `ffx` Development Guide for AI Agents

`ffx` (Fuchsia Command Line Tools) is the primary host-side developer tool for interacting with Fuchsia target devices, product bundles, emulators, and build artifacts. When writing, refactoring, or reviewing code under `//src/developer/ffx`, follow these architecture rules and team best practices.

---

## 1. Subtool Architecture & Code Organization

### External Subtools (`tools/`) over Built-in Plugins (`plugins/`)
* **New subtools belong in `//src/developer/ffx/tools/`** (or in the owning team's subsystem directory with `file:/src/developer/ffx/OWNERS` included in `OWNERS`), built as standalone binaries using the `ffx_tool` GN template (`//src/developer/ffx/build/ffx_tool.gni`).
* **Avoid adding new subtools to `//src/developer/ffx/plugins/`**: The `plugins/` directory is for legacy built-in commands compiled directly into the main `ffx` binary. Only add to `plugins/` if explicitly required.
* **Shared libraries belong in `//src/developer/ffx/lib/`**: Reusable domain logic, target connection handling, protocol wrappers, and configuration schemas should live in `lib/` crates rather than inside individual subtools.

### Split Subtools into `lib.rs` and `main.rs`
Structure every subtool as a library crate (`rustc_library("lib")` with `with_unit_tests = true`) paired with a thin `ffx_tool` binary wrapper:
* **`src/main.rs`**: Minimal entry point invoking FHO:
  ```rust
  use ffx_tool_example::ExampleTool;
  use fho::FfxTool;

  #[fuchsia_async::run_singlethreaded]
  async fn main() {
      ExampleTool::execute_tool().await
  }
  ```
* **`src/lib.rs`**: Defines the `argh` command struct (`#[derive(ArgsInfo, FromArgs, Debug, PartialEq)]`), the `#[derive(FfxTool)]` struct, the `FfxMain` implementation, and unit tests.
* **Rust Edition**: Use `edition = "2024"` in all new `BUILD.gn` targets.

---

## 2. Daemonless Architecture & FDomain Target Connections

`ffx` operates on a **daemonless, direct-connection architecture**.

### Do Not Use Legacy Daemon or Overnet APIs
* **No `ffx-daemon` (`DaemonProxy`)**: Never introduce dependencies on `DaemonProxy`, `fidl_fuchsia_developer_ffx::DaemonProxy`, or `daemon.*` configuration keys.
* **No Host FIDL (`fuchsia.developer.ffx`) for Target/Discovery State**: Do not use legacy `fidl_fuchsia_developer_ffx::TargetInfo` or `TargetProxy`. Use native Rust domain types from:
  * `//src/developer/ffx/lib/discovery` (`TargetHandle`, `TargetEvent`)
  * `//src/developer/ffx/lib/target` (`TargetInfo`, `TargetInfoQuery`)
  * `//src/developer/ffx/lib/mdns_discovery` (`MdnsTargetInfo`)
* **Use FDomain (`*_rust_fdomain`) Instead of Overnet (`*_rust`)**:
  * Target FIDL communication uses **FDomain** (`fdomain_client`) over direct SSH, VSOCK, or USB transports.
  * In `BUILD.gn`, depend on the `_rust_fdomain` target of a FIDL library (e.g., `//sdk/fidl/fuchsia.device:fuchsia.device_rust_fdomain`) and import the `fdomain_fuchsia_*` crate in Rust.

---

## 3. FHO (`//src/developer/ffx/lib/fho`) & Dependency Injection

Subtools use FHO (`FfxTool` and `FfxMain`) to declaratively inject environment context, target proxies, and configuration.

```rust
use argh::{ArgsInfo, FromArgs};
use async_trait::async_trait;
use fdomain_fuchsia_device::NameProviderProxy;
use ffx_writer::{ToolIO as _, VerifiedMachineWriter};
use fho::{FfxContext, FfxMain, FfxTool, Result};
use target_holders::moniker;

#[derive(ArgsInfo, FromArgs, Debug, PartialEq)]
#[argh(subcommand, name = "example", description = "example ffx subtool")]
pub struct ExampleCommand {}

#[derive(FfxTool)]
pub struct ExampleTool {
    #[command]
    cmd: ExampleCommand,
    #[with(moniker("/core/system-update"))]
    proxy: NameProviderProxy,
}
```

### Target Connection Declarations & Holders (`//src/developer/ffx/lib/target/holders`)
* **Tools that do NOT talk to a target device**: Always annotate the `FfxTool` struct with `#[target(None)]`. This is required so `ffx --strict` does not demand a `--target` argument when running the command.
* **Immediate Target Injection**: Use `#[with(moniker("..."))]` or `#[with(toolbox())]` on a FIDL proxy field (or inject `RemoteControlProxyHolder`, `NodenameHolder`, `SshAddrHolder`, `HostAddrHolder`) when every invocation of the tool needs the target.
* **Conditional / Lazy Target Injection (`fho::Deferred<T>`)**: When a tool has subcommands or code paths that may not require a target connection, wrap the proxy or holder in `fho::Deferred<T>` (or `#[with(fho::deferred(moniker("...")))]`) and `.await?` it only on the branch that needs it.
* **Resilient Multi-Shot Reconnection (`Connector<T>`)**: For workflows that reboot or flash the target device and must reconnect across disconnects, use `Connector<RemoteControlProxyHolder>` or `DirectConnector` (`try_connect()`).

---

## 4. Structured Output & Writers (`//src/developer/ffx/lib/writer`)

Never use `println!` or `eprintln!` for tool output. Always write through the `Writer` passed to `FfxMain::main`.

### Use `VerifiedMachineWriter<T>` by Default
New subtools should support `ffx --machine json` with a compile-time JSON schema via `schemars::JsonSchema`:

```rust
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExampleOutput {
    Success { device_name: String },
    Error { message: String },
}

#[async_trait(?Send)]
impl FfxMain for ExampleTool {
    type Writer = VerifiedMachineWriter<ExampleOutput>;

    async fn main(self, mut writer: Self::Writer) -> Result<()> {
        let name = self.proxy.get_device_name().await.user_message("Failed to query device")?;
        writer.machine_or(&ExampleOutput::Success { device_name: name.clone() }, format!("Device: {name}"))?;
        Ok(())
    }
}
```

* **`writer.machine_or(&item, human_text)` / `writer.item(&item)`**: Emits structured JSON in `--machine` mode and human-readable text otherwise.
* **`writer.line(...)` / `writeln!(writer, ...)`**: Emits text only in human mode (no-op in `--machine` mode).
* **Do not abuse `MachineWriter<String>` or `MachineWriter<serde_json::Value>`**: If a command genuinely has no structured output use case, use `SimpleWriter`. Using `MachineWriter<String>` creates an untyped contract.
* **Golden Checks (`cli-goldens` & `mw-goldens`)**:
  * CLI flags (`ArgsInfo`) are verified against `//src/developer/ffx/tests/cli-goldens`.
  * Machine output schemas (`JsonSchema`) are verified against `//src/developer/ffx/tests/mw-goldens`.
  * If you modify CLI arguments or machine output types, run the golden tests and update the golden files as instructed by the build failure.

---

## 5. Error Handling (`//src/developer/ffx/lib/command/error`)

### Moratorium on `anyhow`
* **Do NOT use `anyhow` (`anyhow::Error`, `anyhow::Result`, `anyhow!`, `bail!`, `Context`) in new or refactored `ffx` code**, whether in subtools or library crates.
* **Why**: `anyhow` erases error types, prevents callers from programmatically matching on failure modes, and obscures the critical distinction between actionable user errors and internal tool bugs. When touching existing code that uses `anyhow`, migrate it to typed errors (`thiserror`) or `fho::Result` (`ffx_command_error::Result`) as appropriate.
* **What to use instead**:
  * **In library crates (`//src/developer/ffx/lib/*`)**: Define strongly-typed domain error enums using `#[derive(thiserror::Error, Debug)]`.
  * **In subtools (`//src/developer/ffx/tools/*`)**: Use `fho::Result<T>` and `fho::Error` (`ffx_command_error::Error`), converting library errors at the subtool boundary via `From` or `fho::FfxContext`.

### User Errors vs. Internal Bugs
`ffx` distinguishes between **Actionable User Errors** and **Unexpected Internal Bugs** via `fho::Error` (`ffx_command_error::Error`):
1. **User Errors (`Error::User`)**: Printed cleanly to `stderr` without stack traces. Use for bad CLI arguments, missing files, target unreachability, or anything the user can act on.
2. **Internal Bugs (`Error::Unexpected`)**: Prints a `BUG: An internal command error occurred.` banner with full error chain diagnostics and instructs the user to file a bug at `go/ffx-bug`.

### Best Practices for Error Propagation
* **Use `fho::FfxContext` instead of legacy `ffx_error!` / `ffx_bail!` or `anyhow` macros**:
  * Attach user-facing context with `.user_message("...")` or `.with_user_message(|| format!(...))`:
    ```rust
    let contents = std::fs::read_to_string(&path)
        .with_user_message(|| format!("Unable to read manifest at '{}'", path.display()))?;
    ```
  * Mark internal invariant failures with `.bug()` or `.bug_context("...")`:
    ```rust
    let parsed = parse_internal_state().bug_context("Internal state corrupted")?;
    ```
  * For early returns or standalone errors, use `fho::return_user_error!(...)`, `fho::user_error!(...)`, `fho::return_bug!(...)`, or `fho::bug!(...)`.
* **Actionable Error Messages**: State *what* failed first, followed by *how* the user can resolve it (e.g., `"Target connection failed. Run 'ffx target list' or 'ffx doctor' to verify device state."`).

---

## 6. Configuration (`//src/developer/ffx/config`)

`ffx` resolves configuration across a 5-level priority hierarchy (`ConfigLevel` in `//src/developer/ffx/config`):
1. **`Runtime`** (`--config` / `-c key=val` CLI flags — highest priority)
2. **`User`** (`~/.fuchsia/config.json`)
3. **`Build`** (active build directory configuration, read-only)
4. **`Global`** (system-wide policy configuration)
5. **`Default`** (compiled-in defaults via `include_default!()` — lowest priority)

* **Always Thread `EnvironmentContext` Explicitly**:
  * Never rely on ambient global configuration state when an `EnvironmentContext` can be passed or injected.
  * `EnvironmentContext` implements `TryFromEnv` and can be injected directly as a field on your `#[derive(FfxTool)]` struct:
    ```rust
    #[derive(FfxTool)]
    pub struct ExampleTool {
        #[command]
        cmd: ExampleCommand,
        context: EnvironmentContext,
    }
    ```
* **Querying Configuration via `EnvironmentContext`**:
  * Use `self.context.get::<T, _>("key.path")` or `self.context.get_optional::<T, _>("key.path")` for direct typed lookups, or `self.context.query("key.path")` (`ConfigQueryBuilder`) when specifying a `ConfigLevel` or `SelectMode`.
  * For structured config-backed types, use `#[derive(FfxConfigBacked)]` (`//src/developer/ffx/config/macro`) with `#[ffx_config_default(key = "...", default = "...")]` attributes, or implement `ffx_config::TryFromEnvContext`.
* **Support `ffx --strict`**: Avoid assuming ambient host state or implicit user/build config files exist. Any required settings in strict mode must be resolvable via explicit CLI flags or `-c` runtime config overrides (`EnvironmentContext::is_strict()`).

---

## 7. Testing Guidelines

### Unit Testing Subtools
* **Mock Target Services with Local FDomain Proxies**:
  Use `fdomain_local::local_client_empty()` and `target_holders::fake_proxy` (or `fake_async_proxy`) to test `FfxMain::main` without an emulator or network connection:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use ffx_writer::{Format, TestBuffer};

      fn setup_fake_proxy() -> NameProviderProxy {
          let client = fdomain_local::local_client_empty();
          target_holders::fake_proxy::<NameProviderProxy>(client, move |req| match req {
              fdomain_fuchsia_device::NameProviderRequest::GetDeviceName { responder } => {
                  responder.send(Ok("fuchsia-test-node")).unwrap();
              }
          })
      }

      #[fuchsia::test]
      async fn test_example_json_output() {
          let tool = ExampleTool { cmd: ExampleCommand {}, proxy: setup_fake_proxy() };
          let buffers = TestBuffer::default();
          let writer = VerifiedMachineWriter::<ExampleOutput>::new_buffers(
              Some(Format::Json),
              buffers.clone(),
              Vec::new(),
          );
          tool.main(writer).await.expect("tool should succeed");
          let output = buffers.into_string();
          VerifiedMachineWriter::<ExampleOutput>::verify_schema(&serde_json::from_str(&output).unwrap())
              .expect("output must match schema");
      }
  }
  ```
* **Isolated Config in Tests**: When testing code that reads or writes `ffx_config`, initialize an isolated test environment with `let test_env = ffx_config::test_init().expect("test env");` and pass `&test_env.context`.
* **Registering Host Tests in `BUILD.gn`**:
  Always include a `tests` group in the subtool/library `BUILD.gn` and ensure it is wired into the parent `tests` group (e.g., `//src/developer/ffx/tools/BUILD.gn`):
  ```gn
  group("tests") {
    testonly = true
    deps = [ ":lib_test($host_toolchain)" ]
  }
  ```
* **End-to-End Tests (`ffx_e2e_emu`)**: When an integration test against a real Fuchsia system is necessary, use `//src/developer/ffx/lib/e2e_emu` (`IsolatedEmulator`) and `//src/developer/ffx/lib/isolate` so the test runs in a sandboxed isolate directory without polluting the developer's host environment.
