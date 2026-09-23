//! The subprocess/IPC built-in kind's shim (spec.md §7 kind 3; sessions A11, A12).
//! An IPC built-in is *one external program speaking a JSON protocol*, plus
//! optionally a **bundle** — a directory of files that program needs (a multi-file
//! Python project, a Node.js script and its `node_modules`, a jar, a helper
//! binary). The manifest (`calc_builtins::BindingKind::Ipc`) declares both; nothing
//! here is specific to a language. Spawned fresh per call (spec.md §14.6's simplest
//! lifecycle; a long-lived worker is deferred to Part C).
//!
//! **Protocol (spec.md §14.5)**: one UTF-8 JSON request on the child's stdin,
//! `{"args": [x]}`, read until EOF; one UTF-8 JSON response on its stdout,
//! `{"output": "...", "result": x}`; exit status 0. `output` is text to relay to this
//! process's stdout, `result` the built-in's return value. JSON has no infinity or
//! NaN, so a non-finite number travels as a string (`"inf"`, `"-inf"`, `"NaN"`) in
//! either direction. The child's stderr is free for its own diagnostics.
//!
//! **Where the bundle comes from** ([`BundleSource`]): a compiled program carries its
//! bundles inside the executable, as data the link driver embedded
//! (`calc_ipc_bundles`), unpacked on first use into a per-user cache directory; the
//! interpreter runs a bundle straight from its source directory. Same program, same
//! files, only the location differs.
//!
//! **Failure semantics (spec.md §14.7)**: `calcc build` already checked the command
//! runs, so a failure here — the command missing on this machine, a non-zero exit, an
//! invalid response, an unwritable cache — is a `panic!` with a message saying which.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use calc_builtins::bundle_format::{self, Bundle};
use calc_builtins::BindingKind;
use serde_json::{json, Value};

/// Where an IPC built-in's bundle is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleSource {
    /// Embedded in this executable by the link driver: compiled programs.
    Embedded,
    /// `calc-runtime/ipc/<bundle>` in the source tree: the interpreter and tests.
    Source,
}

extern "C" {
    // The packed bundles (`calc_builtins::bundle_format`) and their length, defined in
    // a compiled program by the data object the link driver generates, and in Rust
    // consumers by `native/no_ipc_bundles.c` (an empty table).
    static calc_ipc_bundles: u8;
    static calc_ipc_bundles_len: u64;
}

/// This platform's `target_family`, the key the manifest's `IpcCommand::only_on`
/// uses. `calc-runtime` is always compiled for the target it runs on, so `cfg!` is
/// the answer, not a guess.
const TARGET_FAMILY: &str = if cfg!(windows) { "windows" } else { "unix" };

/// What the program sent back.
#[derive(Debug, PartialEq)]
struct Response {
    output: String,
    result: f64,
}

/// Runs the IPC built-in `name` with argument `x`, relays its `output` to this
/// process's stdout — calc-lang's program output, beyond a final numeric result —
/// and returns its `result`.
pub fn call(name: &str, x: f64, source: BundleSource) -> f64 {
    let response = run(name, x, source);
    print!("{}", response.output);
    response.result
}

/// Split out of `call` so a test can assert on the response directly, rather than
/// needing to capture this process's own stdout.
fn run(name: &str, x: f64, source: BundleSource) -> Response {
    let builtin = calc_builtins::lookup(name)
        .unwrap_or_else(|| panic!("calc-runtime: no built-in `{name}` in the manifest"));
    let BindingKind::Ipc { bundle, .. } = builtin.kind else {
        panic!("calc-runtime: `{name}` is not an IPC built-in");
    };
    let command = builtin.ipc_command(TARGET_FAMILY).unwrap_or_else(|| {
        panic!("calc-runtime: the manifest declares no {TARGET_FAMILY} command for `{name}`")
    });
    let bundle_dir = match bundle {
        Some(bundle) => bundle_dir(bundle, source),
        None => PathBuf::new(),
    };
    let resolved = command.resolve(&bundle_dir.display().to_string());
    let request = json!({ "args": [encode(x)] }).to_string();
    let stdout = spawn(&resolved.program, &resolved.args, &resolved.env, &request)
        .unwrap_or_else(|err| panic!("calc-runtime: built-in `{name}` failed: {err}"));
    parse_response(&stdout).unwrap_or_else(|| {
        panic!("calc-runtime: built-in `{name}` sent an invalid response: {stdout}")
    })
}

/// Starts `program`, writes `request` to its stdin, and returns its stdout if it
/// exited successfully.
fn spawn(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    request: &str,
) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("couldn't start `{program}`: {err}"))?;
    // Taking stdin out of `child` and dropping it after the write closes the pipe,
    // which is how the program knows the request is complete. A write error means the
    // child already exited; its exit status, checked below, says why.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(request.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("`{program}` didn't finish: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{program}` exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("`{program}` wrote non-UTF-8 output"))
}

/// The directory to run bundle `name` from.
fn bundle_dir(name: &str, source: BundleSource) -> PathBuf {
    match source {
        BundleSource::Source => Path::new(env!("CARGO_MANIFEST_DIR")).join("ipc").join(name),
        BundleSource::Embedded => {
            let bundles = bundle_format::parse(embedded_blob())
                .expect("calc-runtime: the embedded IPC bundle table is malformed");
            let bundle = bundles
                .iter()
                .find(|b| b.name == name)
                .unwrap_or_else(|| panic!("calc-runtime: bundle `{name}` isn't embedded"));
            unpack(bundle, &cache_root()).unwrap_or_else(|err| {
                panic!("calc-runtime: couldn't unpack bundle `{name}`: {err}")
            })
        }
    }
}

/// The packed bundles embedded in this executable.
fn embedded_blob() -> &'static [u8] {
    // SAFETY: both symbols are defined together, either by the link driver's data
    // object or by `native/no_ipc_bundles.c`, and the first is at least
    // `calc_ipc_bundles_len` bytes long.
    unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!(calc_ipc_bundles),
            calc_ipc_bundles_len as usize,
        )
    }
}

/// Where bundles are unpacked: `CALC_BUNDLE_CACHE` if set (an explicit override,
/// e.g. for a machine whose home directory is mounted `noexec`), otherwise the
/// platform's per-user cache directory. Per-user rather than a shared temp directory,
/// so another user can't plant files at the predictable path.
fn cache_root() -> PathBuf {
    if let Some(dir) = env::var_os("CALC_BUNDLE_CACHE") {
        return PathBuf::from(dir);
    }
    let dir = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))
    };
    dir.unwrap_or_else(|| {
        panic!("calc-runtime: no per-user cache directory found; set CALC_BUNDLE_CACHE")
    })
}

/// Unpacks `bundle` into `<root>/calc-bundles/<name>-<hash>/`, unless it's already
/// there, and returns that directory. The name includes the content hash, so an
/// existing directory is never stale. Unpacking writes to a temporary sibling and
/// renames it into place, so a concurrent first run can't see a half-written bundle.
fn unpack(bundle: &Bundle, root: &Path) -> std::io::Result<PathBuf> {
    let parent = root.join("calc-bundles");
    let dir = parent.join(format!("{}-{:016x}", bundle.name, bundle.hash));
    if dir.is_dir() {
        return Ok(dir);
    }
    let staging = parent.join(format!(
        ".{}-{:016x}.{}",
        bundle.name,
        bundle.hash,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&staging);
    for file in &bundle.files {
        let path = staging.join(&file.path);
        fs::create_dir_all(path.parent().expect("a file path has a parent"))?;
        fs::write(&path, &file.bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(file.mode))?;
        }
    }
    fs::create_dir_all(&staging)?;
    match fs::rename(&staging, &dir) {
        Ok(()) => Ok(dir),
        // Another process unpacked the same bundle first; use theirs.
        Err(_) if dir.is_dir() => {
            let _ = fs::remove_dir_all(&staging);
            Ok(dir)
        }
        Err(err) => Err(err),
    }
}

fn parse_response(text: &str) -> Option<Response> {
    let value: Value = serde_json::from_str(text).ok()?;
    Some(Response {
        output: value.get("output")?.as_str()?.to_string(),
        result: decode(value.get("result")?)?,
    })
}

/// A number as JSON: a JSON number when finite, otherwise Rust's own spelling as a
/// string (`inf`, `-inf`, `NaN`), which Python's `float()` also accepts.
fn encode(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        json!(x.to_string())
    }
}

/// The inverse of [`encode`], also accepting Python's spellings (`inf`, `nan`).
fn decode(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use calc_builtins::bundle_format::{pack, parse, BundleFile};

    use super::*;

    /// Runs the real multi-file `ipc/print` project, not a Rust reimplementation of
    /// its formatting. No `\r\n` normalization needed: the newline travels escaped
    /// inside a JSON string, so the platform's text-mode stdout never touches it.
    #[test]
    fn runs_the_multi_file_project_over_the_json_protocol() {
        assert_eq!(
            run("print", 12.3456, BundleSource::Source),
            Response {
                output: "result = 12.35\n".to_string(),
                result: 12.3456,
            }
        );
    }

    #[test]
    fn non_finite_numbers_round_trip() {
        let response = run("print", f64::INFINITY, BundleSource::Source);
        assert_eq!(response.output, "result = inf\n");
        assert_eq!(response.result, f64::INFINITY);
        assert!(run("print", f64::NAN, BundleSource::Source).result.is_nan());
    }

    /// Rust consumers link the empty table from `native/no_ipc_bundles.c`.
    #[test]
    fn a_rust_consumer_embeds_no_bundles() {
        assert_eq!(parse(embedded_blob()), Some(Vec::new()));
    }

    #[test]
    fn unpacks_a_bundle_once_into_a_content_named_directory() {
        let blob = pack(&[(
            "demo",
            vec![
                BundleFile {
                    path: "__main__.py".into(),
                    mode: 0o644,
                    bytes: b"print(1)".to_vec(),
                },
                BundleFile {
                    path: "pkg/helper".into(),
                    mode: 0o755,
                    bytes: b"#!/bin/sh".to_vec(),
                },
            ],
        )]);
        let bundle = &parse(&blob).unwrap()[0];
        let root = env::temp_dir().join(format!("calc_a12_unpack_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let dir = unpack(bundle, &root).unwrap();
        assert!(dir.ends_with(format!("demo-{:016x}", bundle.hash)));
        assert_eq!(fs::read(dir.join("pkg/helper")).unwrap(), b"#!/bin/sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("pkg/helper"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        // A second run reuses the directory rather than rewriting it.
        fs::write(dir.join("marker"), b"").unwrap();
        assert_eq!(unpack(bundle, &root).unwrap(), dir);
        assert!(dir.join("marker").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failing_program_is_reported_with_its_exit_status() {
        let err = spawn("calc-no-such-program", &[], &[], "{}").unwrap_err();
        assert!(
            err.starts_with("couldn't start `calc-no-such-program`"),
            "{err}"
        );
    }

    #[test]
    fn rejects_malformed_responses() {
        assert_eq!(parse_response("not json"), None);
        assert_eq!(parse_response(r#"{"output": "x"}"#), None);
    }

    #[test]
    fn call_returns_the_programs_result() {
        assert_eq!(call("print", 2.5, BundleSource::Source), 2.5);
    }
}
