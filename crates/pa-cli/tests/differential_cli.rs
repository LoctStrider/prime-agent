// large_futures: stack futures on hot paths by design. too_many_lines: style gate
// only. Casts: 64-bit targets; narrowing sits at bounded OS/protocol boundaries.
#![allow(
    clippy::large_futures,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

//! Differential CLI tests: run a corpus of invocations against both the
//! explicitly configured TypeScript v0.9.8 binary (compatibility reference) and
//! the Rust binary, and assert matching exit codes, stdout, and stderr.
//! Corpus policy: only invocations fully determined by the CLI surface
//! itself (help/version, validation errors, routing errors, MCP settings
//! reads/writes); skipped only when PA_TS_BINARY is not configured.
#![cfg(unix)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const CORPUS: &[&[&str]] = &[
    // Help and version.
    &["--version"],
    &["-v"],
    &["--help"],
    &["-h"],
    &["help"],
    &["help", "help"],
    &["help", "agents"],
    &["help", "list"],
    &["help", "sessions"],
    &["help", "attach"],
    &["help", "stop"],
    &["help", "rename"],
    &["help", "send"],
    &["help", "schedule"],
    &["help", "schedule", "list"],
    &["help", "schedule", "add"],
    &["help", "schedule", "cancel"],
    &["help", "status"],
    &["help", "doctor"],
    &["help", "shutdown"],
    &["help", "mcp"],
    &["help", "mcp", "add"],
    &["help", "mcp", "list"],
    &["help", "mcp", "get"],
    &["help", "mcp", "remove"],
    &["help", "package"],
    &["help", "package", "install"],
    &["help", "package", "remove"],
    &["help", "package", "list"],
    &["help", "package", "update"],
    &["help", "update"],
    &["help", "model"],
    &["help", "model", "list"],
    &["help", "session"],
    &["help", "session", "export"],
    &["help", "config"],
    // Typos produce suggestions.
    &["help", "schedul"],
    &["help", "schedule", "bogus"],
    &["help", "help", "help"],
    // Argument parser diagnostics.
    &["-x"],
    &["-x", "--thinking", "bogus"],
    &["--mode"],
    &["--mode", "bogus"],
    &["--thinking", "bogus"],
    &["--thinking", "off", "--thinking", "bogus"],
    &["--provider"],
    &["--model"],
    &["--api-key"],
    &["--cwd"],
    &["--system-prompt"],
    &["--fork"],
    &["--session-dir"],
    &["--models"],
    &["--goal-token-budget", "5"],
    &["--goal", ""],
    &["--goal", "  "],
    &["--autonomous-max-turns", "0"],
    &["--autonomous-max-turns", "abc"],
    &["--autonomous-max-tokens", "-1"],
    &["--export", "foo"],
    &["--export=foo"],
    &["--list-models"],
    &["--list-models", "gpt"],
    &["--list-models=gpt"],
    // Daemon client connect failure against a socket that never exists: the
    // full error text is deterministic.
    &[
        "--daemon-socket",
        "/nonexistent-pa-daemon-differential.sock",
        "list",
    ],
    &[
        "--daemon-socket",
        "/nonexistent-pa-daemon-differential.sock",
        "list",
        "--json",
    ],
    &[
        "--daemon-socket",
        "/nonexistent-pa-daemon-differential.sock",
        "sessions",
    ],
    &[
        "--daemon-socket",
        "/nonexistent-pa-daemon-differential.sock",
        "sessions",
        "--all",
        "--json",
    ],
    &[
        "--daemon-socket",
        "/nonexistent-pa-daemon-differential.sock",
        "stop",
        "some-agent",
    ],
    // Flag interdependency validation.
    &["--fork", "x", "--continue"],
    &["--fork", "x", "--resume", "y"],
    &["--fork", "x", "--no-session"],
    &["--resume"],
    &["--resume="],
    &["--mode", "rpc", "@file.txt"],
    &["--mode", "daemon", "@file.txt"],
    &[
        "--cwd",
        "/nonexistent-differential-test-dir",
        "--mode",
        "json",
    ],
    // Removed commands.
    &["daemon"],
    &["daemon", "foo"],
    &["app"],
    &["app", "update"],
    &["install"],
    &["remove"],
    &["uninstall"],
    &["manage"],
    &["manage", "update"],
    // Daemon discovery against an empty sandbox state root: deterministic
    // output for both binaries.
    &["status"],
    &["status", "--json"],
    &["doctor"],
    &["doctor", "--json"],
    &["doctor", "--fix"],
    &["doctor", "--fix", "--json"],
    &["shutdown"],
    &["shutdown", "--json"],
    &["shutdown", "--force", "--json"],
    // Public command routing and validation.
    &["--offline", "status"],
    &["--verbose", "status", "--json"],
    &["--help", "status"],
    &["status", "--bogus"],
    &["status", "--offline"],
    &["doctor", "--bogus"],
    &["shutdown", "--bogus"],
    &["stop"],
    &["stop", "a", "b"],
    &["rename", "a"],
    &["send", "--help"],
    &["schedule"],
    &["schedule", "bogus"],
    &["schedule", "list", "--bogus"],
    &["schedule", "list", "a", "b"],
    &["schedule", "cancel"],
    &["schedule", "list", "--help"],
    &["attach"],
    &["attach", "--resume"],
    &["attach", "a", "b"],
    &["attach", "a", "--resume"],
    &["attach", "a", "-r"],
    &["attach", "a", "--resume=x"],
    &["attach", "a", "--continue"],
    &["attach", "a", "--fork"],
    // model/session command rewrites.
    &["model"],
    &["model", "bogus"],
    &["model", "list", "x", "y"],
    // Model catalog rows (`model list`): these need provider auth visible to
    // both binaries so the catalog, not the no-models guidance, prints;
    // PI_OFFLINE keeps it deterministic (bundled, no network refresh).
    &["model", "list"],
    &["model", "list", "gpt"],
    &["model", "list", "claude"],
    &["model", "list", "z-ai"],
    &["model", "list", "z-ai", "glm"],
    &["model", "list", "zzz-nomatch-xyz"],
    &["model", "list", "--json"],
    &["session"],
    &["session", "bogus"],
    &["session", "export"],
    &["session", "export", "a", "b", "c"],
    &["session", "export", "--x"],
    // config command validation (the interactive view itself is excluded:
    // it needs a terminal).
    &["config", "x"],
    // update command validation.
    &["update", "--self"],
    &["update", "package"],
    &["update", "--extensions"],
    &["update", "--self", "package"],
    &["update", "self", "pkg"],
    &["update", "--bogus"],
    &["update", "--force", "--self"],
    // package command validation.
    &["package"],
    &["package", "bogus"],
    &["package", "uninstall"],
    &["package", "list", "x"],
    &["package", "update", "--self"],
    &["package", "update", "a", "b"],
    &["package", "update", "self"],
    &["package", "update", "prime-agent"],
    &["package", "update", "pi"],
    &["package", "update", "--force"],
    &["package", "install"],
    &["package", "remove"],
    &["package", "install", "--bogus"],
    &["package", "install", "-l"],
    &["package", "install", "-h"],
    &["package", "remove", "-h"],
    &["package", "list", "-h"],
    &["package", "update", "-h"],
    &["package", "update", "--nightly", "--stable"],
    &["package", "update", "--extension", "x", "--self"],
    &["package", "update", "--extension", "x", "y"],
    &["package", "update", "--extensions", "--rollback"],
    &["package", "update", "--stable", "some-source"],
    // MCP management (stateful cases share one sandbox HOME per binary).
    &["mcp"],
    &["mcp", "bogus"],
    &["mcp", "list", "x"],
    &["mcp", "list"],
    &["mcp", "get"],
    &["mcp", "get", "x"],
    &["mcp", "get", "bad name!"],
    &["mcp", "remove", "x"],
    &["mcp", "add"],
    &["mcp", "add", "bad name!"],
    &["mcp", "add", "linear"],
    &["mcp", "add", "x"],
    &["mcp", "add", "x", "--url", "notaurl"],
    &["mcp", "add", "x", "--url", "ftp://foo"],
    &["mcp", "add", "x", "--url", "http://u:p@host"],
    &["mcp", "add", "x", "--url", "http://"],
    &["mcp", "add", "x", "--url"],
    &["mcp", "add", "x", "--bogus", "v"],
    &[
        "mcp",
        "add",
        "x",
        "--url",
        "https://e.com",
        "--url",
        "https://f.com",
    ],
    &[
        "mcp",
        "add",
        "x",
        "--url",
        "https://e.com",
        "--oauth",
        "--bearer-token-env-var",
        "V",
    ],
    &["mcp", "add", "x", "--env", "CHILD"],
    &["mcp", "add", "x", "--env", "CHILD=bad-name"],
    &["mcp", "add", "x", "--env", "=SOURCE"],
    &["mcp", "add", "x", "--cwd", "/tmp", "--url", "https://e.com"],
    &[
        "mcp",
        "add",
        "x",
        "--url",
        "https://e.com",
        "--",
        "run",
        "arg",
    ],
    &["mcp", "add", "y", "--url", "https://example.com/mcp"],
    &["mcp", "list"],
    &["mcp", "get", "y"],
    &["mcp", "add", "y", "--url", "https://example.com/mcp"],
    &[
        "mcp",
        "add",
        "y",
        "--force",
        "--url",
        "https://other.com/mcp",
    ],
    &["mcp", "remove", "y"],
    &["mcp", "list"],
    &["mcp", "remove", "y"],
];

fn ts_binary() -> Option<PathBuf> {
    configured_ts_binary(std::env::var_os("PA_TS_BINARY"))
}

fn configured_ts_binary(path: Option<std::ffi::OsString>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    let path = path?;
    assert!(!path.is_empty(), "PA_TS_BINARY must not be empty");
    let path = std::fs::canonicalize(&path)
        .unwrap_or_else(|error| panic!("invalid PA_TS_BINARY {path:?}: {error}"));
    let metadata = std::fs::metadata(&path).expect("stat PA_TS_BINARY");
    assert!(
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        "PA_TS_BINARY must be an executable file: {}",
        path.display()
    );
    Some(path)
}

struct InvocationOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(binary: &Path, args: &[&str], sandbox: &Path) -> InvocationOutput {
    run_with_env(binary, args, sandbox, &[])
}

/// Normalize version numbers so the TS product version (0.9.x) and the Rust
/// workspace version can be compared structurally.
fn normalize(text: &str, sandbox_roots: &[&Path]) -> String {
    let mut text = text.to_string();
    // Sandbox paths leak into cwd-related error messages; normalize them so
    // the per-binary sandbox roots compare equal.
    for root in sandbox_roots {
        text = text.replace(&root.display().to_string(), "<SANDBOX>");
    }
    // Docs paths in login guidance resolve to each binary's own install dir
    // (package dir); compare the shape, not the installation location.
    text = normalize_docs_paths(&text);
    normalize_versions(&text)
}

/// Compare only the documented cross-version changes, and only in their help
/// contexts. Unknown/edited rows remain visible to the byte comparison.
fn compatible_help(text: &str, args: &[&str]) -> String {
    let mut text = text.to_string();
    if matches!(args, ["--help"] | ["-h"] | ["help"]) {
        // #3189 removed extensions; #3211 removed no-op tool-selection flags.
        text = text.replace("Tool and resource options:\n", "Resource options:\n");
        for row in [
            "  -t, --tools <list>          Allowlist comma-separated tool names\n",
            "  -nt, --no-tools             Disable all tools by default\n",
            "  -nbt, --no-builtin-tools    Disable built-in tools by default\n",
            "  -e, --extension <source>    Load an extension (repeatable)\n",
            "  -ne, --no-extensions        Disable extension discovery\n",
            // #3285 machine library; the prompt dump is a Rust-first addition.
            "  factory   Share and run factory machines from the machine library\n",
            "  prompt    Print the assembled system prompt with its layer breakdown\n",
        ] {
            text = text.replace(row, "");
        }
        // #2524 replaced self-update with the Rust installer funnel.
        text = text.replace(
            "  update    Update to the latest Rust build (uninstalls the TypeScript version)\n",
            "  update    Update Prime Agent\n",
        );
    }
    if args == ["help", "package"] {
        text = text.replace(
            "Packages can provide extensions, skills, prompts, and themes.",
            "Packages can provide skills, prompts, and themes.",
        );
    }
    if args == ["help", "update"] {
        // #2524 installer funnel, #3243 channel selection, #3278 local rollback/archive.
        // Map exact changed rows, leaving the shared scaffolding under comparison.
        for (rust, ts) in [
            ("  prime-agent update [--check] [--nightly|--stable]\n", "  prime-agent update [--force] [--rollback] [--nightly|--stable]\n"),
            ("Update to the latest Rust build (uninstalls the TypeScript version).\n", "Update Prime Agent.\n"),
            ("  --force     Reinstall the latest build of the channel\n", "  --force     Reinstall even if the current version is the latest on the channel\n"),
            ("  --rollback  Restore the previous version the last update kept\n", "  --rollback  Restore the previous compiled release\n"),
            ("  --nightly   Switch updates to the nightly channel (the latest main build)\n", "  --nightly   Switch updates to the nightly channel (unreleased builds, may be broken)\n"),
            ("Move from the TypeScript version to the Rust port in one step: `prime-agent update` fetches the update channel's installer (stable: https://app.primeintellect.ai/prime-agent/install.sh; nightly: install-beta.sh from the release download base — never a GitHub raw or workflow URL) and runs it, which uninstalls the TypeScript version and installs the latest Rust build; your sessions and configuration (~/.prime/agent) are never touched. Restart prime-agent after the update to run the new build. This command exists only in the Rust binary — the TypeScript version does not have it; the move happens when you run the installer's curl|sh URL (the README's Install section) or `prime-agent update` (after the Rust install exists).\n\n", ""),
            ("  --check  Print the update channel's latest release vs the running version, without installing\n", ""),
            ("  --archive <path>  Install a local release archive\n", ""),
            ("  --source <url>     The https:// origin recorded as the release's install source (managed installs; required there with --archive)\n", ""),
        ] {
            text = text.replace(rust, ts);
        }
    }
    text
}

/// Replace `<any dir>/docs/providers.md|models.md` lines with a placeholder.
fn normalize_docs_paths(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim_start();
            for doc in ["/docs/providers.md", "/docs/models.md"] {
                if trimmed.contains(doc) {
                    return line.replace(trimmed.trim_end(), &format!("<PACKAGE_DOCS>{doc}"));
                }
            }
            line.to_string()
        })
        .collect()
}

fn normalize_versions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let token = &text[start..i];
            let trailing = token.ends_with('.');
            let version_candidate = token.trim_end_matches('.');
            let parts: Vec<&str> = version_candidate.split('.').collect();
            if parts.len() >= 2
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            {
                let _ = write!(out, "X{}.X.X", if trailing { "." } else { "" });
            } else {
                // Non-version digit runs are kept verbatim: trimming here
                // would corrupt adjacent separators (e.g. git fetch ranges).
                out.push_str(token);
            }
        } else {
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn sandbox(prefix: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "pa-cli-differential-{prefix}-{}",
        std::process::id()
    ));
    for dir in ["home", "cwd", "agent", "tmp"] {
        std::fs::create_dir_all(base.join(dir)).expect("create sandbox directory");
    }
    base
}

#[test]
fn differential_corpus_matches_ts_binary() {
    let Some(ts) = ts_binary() else {
        eprintln!("SKIPPED: TS prime-agent binary not found (set PA_TS_BINARY)");
        return;
    };
    let rust = PathBuf::from(env!("CARGO_BIN_EXE_prime-agent"));
    let ts_sandbox = sandbox("ts");
    let rs_sandbox = sandbox("rs");
    let sandbox_roots: Vec<&Path> = vec![&ts_sandbox, &rs_sandbox];

    let mut failures: Vec<String> = Vec::new();
    for case in CORPUS {
        let ts_out = run(&ts, case, &ts_sandbox);
        let rs_out = run(&rust, case, &rs_sandbox);
        // #3189 removed unknown long-option passthrough (the extension flag
        // surface). `--json` was never a model-list format in TS v0.9.8.
        if *case == ["model", "list", "--json"] {
            assert_eq!(rs_out.exit_code, Some(1));
            assert_eq!(rs_out.stdout, "");
            assert_eq!(rs_out.stderr, "Error: Unknown option: --json\n");
            let ts_plain = run(&ts, &["model", "list"], &ts_sandbox);
            assert_eq!(ts_out.exit_code, ts_plain.exit_code);
            assert_eq!(ts_out.stdout, ts_plain.stdout);
            assert_eq!(ts_out.stderr, ts_plain.stderr);
            continue;
        }
        let ok = ts_out.exit_code == rs_out.exit_code
            && normalize(&compatible_help(&ts_out.stdout, case), &sandbox_roots)
                == normalize(&compatible_help(&rs_out.stdout, case), &sandbox_roots)
            && normalize(&compatible_help(&ts_out.stderr, case), &sandbox_roots)
                == normalize(&compatible_help(&rs_out.stderr, case), &sandbox_roots);
        if !ok {
            failures.push(format!(
                "argv: {:?}\n  ts exit {:?} rs exit {:?}\n  ts stdout: {:?}\n  rs stdout: {:?}\n  ts stderr: {:?}\n  rs stderr: {:?}",
                case,
                ts_out.exit_code,
                rs_out.exit_code,
                ts_out.stdout,
                rs_out.stdout,
                ts_out.stderr,
                rs_out.stderr
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "differential corpus mismatches ({} of {}):\n{}",
        failures.len(),
        CORPUS.len(),
        failures.join("\n---\n")
    );
}

fn run_with_env(
    binary: &Path,
    args: &[&str],
    sandbox: &Path,
    envs: &[(&str, &str)],
) -> InvocationOutput {
    let mut command = Command::new("timeout");
    command
        .arg("20")
        .arg(binary)
        .args(args)
        .env_remove("PI_PACKAGE_DIR")
        .env("HOME", sandbox.join("home"))
        .env("TMPDIR", sandbox.join("tmp"))
        .env("PRIME_AGENT_CODING_AGENT_DIR", sandbox.join("agent"))
        .env("PI_OFFLINE", "1")
        .current_dir(sandbox.join("cwd"));
    // Do not let the hosting agent redirect resources, session files, or daemon
    // clients out of the sandbox. The TS wrapper may set its own package root.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("PRIME_AGENT_INTERNAL_")
            || matches!(
                name.as_ref(),
                "PRIME_AGENT_SESSION_DIR"
                    | "PRIME_AGENT_CODING_AGENT_SESSION_DIR"
                    | "PRIME_AGENT_DAEMON_SOCKET"
                    | "RLM_SESSION_DIR"
                    | "RLM_HARNESS_STATE_DIR"
            )
        {
            command.env_remove(key);
        }
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("failed to spawn binary under test");
    InvocationOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn differential_env_flag_cases_match_ts_binary() {
    let Some(ts) = ts_binary() else {
        eprintln!("SKIPPED: TS prime-agent binary not found (set PA_TS_BINARY)");
        return;
    };
    let rust = PathBuf::from(env!("CARGO_BIN_EXE_prime-agent"));
    let ts_sandbox = sandbox("ts-env");
    let rs_sandbox = sandbox("rs-env");
    let sandbox_roots: Vec<&Path> = vec![&ts_sandbox, &rs_sandbox];

    // PI_STARTUP_BENCHMARK is rejected outside interactive mode.
    for case in [&["-p", "hi"][..], &["--print"][..], &["--mode", "json"][..]] {
        let ts_out = run_with_env(&ts, case, &ts_sandbox, &[("PI_STARTUP_BENCHMARK", "1")]);
        let rs_out = run_with_env(&rust, case, &rs_sandbox, &[("PI_STARTUP_BENCHMARK", "1")]);
        assert_eq!(
            ts_out.exit_code, rs_out.exit_code,
            "case {case:?} exit code"
        );
        assert_eq!(
            normalize(&ts_out.stderr, &sandbox_roots),
            normalize(&rs_out.stderr, &sandbox_roots),
            "case {case:?} stderr"
        );
    }
}

fn fixture_session() -> String {
    [
        r#"{"type":"session","id":"d7f6e5c4","version":3,"timestamp":"2026-09-01T10:00:00.000Z","cwd":"/tmp/project"}"#,
        r#"{"type":"message","id":"e1","parentId":null,"timestamp":"2026-09-01T10:00:01.000Z","message":{"role":"user","content":"export a fixture"}}"#,
        r#"{"type":"message","id":"e2","parentId":"e1","timestamp":"2026-09-01T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"the answer"}],"usage":{"inputTokens":10,"outputTokens":5}}}"#,
        r#"{"type":"message","id":"e3","parentId":"e2","timestamp":"2026-09-01T10:00:03.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"tc1","name":"bash","arguments":{"command":"ls"}}]}}"#,
        r#"{"type":"message","id":"e4","parentId":"e3","timestamp":"2026-09-01T10:00:04.000Z","message":{"role":"toolResult","toolCallId":"tc1","toolName":"bash","content":[{"type":"text","text":"file.txt"}],"isError":false}}"#,
        r#"{"type":"label","id":"e5","parentId":"e4","targetId":"e1","label":"start","timestamp":"2026-09-01T10:00:05.000Z"}"#,
    ]
    .join("\n")
    + "\n"
}

/// The base64-embedded session data of an exported file, decoded and parsed
/// (the wire contract is the decoded JSON, not the base64 bytes).
fn exported_session_data(html: &str) -> serde_json::Value {
    use base64::Engine as _;
    let marker = "session-data\" type=\"application/json\">";
    let start = html.find(marker).expect("session data element");
    let blob = &html[start + marker.len()..];
    let blob = blob.split('<').next().expect("script close");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(blob.trim())
        .expect("base64 session data");
    serde_json::from_slice(&decoded).expect("session data JSON")
}

/// The theme CSS custom properties as a sorted set (the TS exporter emits
/// them in file order, the Rust one sorted - the values are the contract).
fn exported_css_vars(html: &str) -> Vec<String> {
    let mut vars: Vec<String> = html
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("--") && line.ends_with(';'))
        .map(str::to_string)
        .collect();
    vars.sort();
    vars
}

/// Differential parity for `session export`: the same fixture exported by
/// both binaries prints the same success line and produces files carrying
/// the same session data, theme CSS variables, and template scaffolding.
#[test]
fn differential_session_export_matches_ts_binary() {
    let Some(ts) = ts_binary() else {
        eprintln!("SKIPPED: TS prime-agent binary not found (set PA_TS_BINARY)");
        return;
    };
    let rust = PathBuf::from(env!("CARGO_BIN_EXE_prime-agent"));
    let ts_sandbox = sandbox("ts-export");
    let rs_sandbox = sandbox("rs-export");
    let sandbox_roots: Vec<&Path> = vec![&ts_sandbox, &rs_sandbox];
    let fixture = "fixture-session.jsonl";
    for sandbox in [&ts_sandbox, &rs_sandbox] {
        std::fs::write(sandbox.join("cwd").join(fixture), fixture_session())
            .expect("write fixture");
    }

    let ts_out = run(
        &ts,
        &["session", "export", fixture, "out.html"],
        &ts_sandbox,
    );
    let rs_out = run(
        &rust,
        &["session", "export", fixture, "out.html"],
        &rs_sandbox,
    );
    assert_eq!(ts_out.exit_code, rs_out.exit_code, "exit codes");
    assert_eq!(
        normalize(&ts_out.stdout, &sandbox_roots),
        normalize(&rs_out.stdout, &sandbox_roots),
        "stdout"
    );
    assert_eq!(
        normalize(&ts_out.stderr, &sandbox_roots),
        normalize(&rs_out.stderr, &sandbox_roots),
        "stderr"
    );
    assert!(
        rs_out.stderr.contains("Exported to: out.html"),
        "success line: {}",
        rs_out.stderr
    );

    let ts_html =
        std::fs::read_to_string(ts_sandbox.join("cwd").join("out.html")).expect("TS export");
    let rs_html =
        std::fs::read_to_string(rs_sandbox.join("cwd").join("out.html")).expect("Rust export");

    // Same session data: header, entries, leaf.
    assert_eq!(
        exported_session_data(&ts_html),
        exported_session_data(&rs_html)
    );
    // Same theme CSS custom properties.
    assert_eq!(exported_css_vars(&ts_html), exported_css_vars(&rs_html));
    // Same template scaffolding (the product export template, verbatim).
    for marker in [
        "<title>Session Export</title>",
        "hamburger",
        "marked",
        "hljs",
    ] {
        assert!(ts_html.contains(marker), "TS export has {marker}");
        assert!(rs_html.contains(marker), "Rust export has {marker}");
    }

    // The missing-file error is the same surface too.
    let ts_missing = run(&ts, &["session", "export", "nope.jsonl"], &ts_sandbox);
    let rs_missing = run(&rust, &["session", "export", "nope.jsonl"], &rs_sandbox);
    assert_eq!(ts_missing.exit_code, rs_missing.exit_code);
    assert_eq!(
        normalize(&ts_missing.stderr, &sandbox_roots),
        normalize(&rs_missing.stderr, &sandbox_roots),
        "missing-file stderr"
    );
}

#[test]
fn ts_reference_requires_explicit_executable() {
    use std::os::unix::fs::PermissionsExt as _;

    assert_eq!(configured_ts_binary(None), None);
    let root = sandbox("reference-validation");
    let file = root.join("reference");
    std::fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
    for invalid in [
        std::ffi::OsString::new(),
        root.as_os_str().to_owned(),
        file.as_os_str().to_owned(),
        root.join("missing").into_os_string(),
    ] {
        assert!(std::panic::catch_unwind(|| configured_ts_binary(Some(invalid))).is_err());
    }
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        configured_ts_binary(Some(file.clone().into_os_string())),
        Some(std::fs::canonicalize(file).unwrap())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn help_exceptions_preserve_shared_text_and_whitespace() {
    let ts = "Usage:\nTool and resource options:\n  -e, --extension <source>    Load an extension (repeatable)\n  --skill <path>              Load a skill (repeatable)\n";
    let rust =
        "Usage:\nResource options:\n  --skill <path>              Load a skill (repeatable)\n";
    assert_eq!(compatible_help(ts, &["help"]), rust);
    assert_ne!(
        compatible_help(ts, &["help"]),
        rust.replace("Load a skill", "Load skills")
    );
    assert_ne!(
        compatible_help(ts, &["help"]),
        rust.replace("  --skill", " --skill")
    );
    assert_eq!(compatible_help(ts, &["help", "mcp"]), ts);
    assert_ne!(normalize(rust, &[]), normalize(rust.trim_end(), &[]));
}

#[test]
fn rust_help_describes_supported_resources_and_installer() {
    let root = sandbox("rust-help");
    let rust = PathBuf::from(env!("CARGO_BIN_EXE_prime-agent"));
    let top = run(&rust, &["help"], &root);
    assert_eq!(top.exit_code, Some(0));
    assert!(top.stderr.is_empty());
    for command in ["factory", "prompt"] {
        assert!(top
            .stdout
            .lines()
            .any(|line| line.starts_with(&format!("  {command} "))));
        let command_help = run(&rust, &["help", command], &root);
        assert_eq!(command_help.exit_code, Some(0));
        assert!(command_help.stdout.starts_with("Usage:\n"));
    }
    for flag in [
        "--tools",
        "--no-tools",
        "--no-builtin-tools",
        "--extension",
        "--no-extensions",
    ] {
        let output = run(&rust, &[flag], &root);
        assert_eq!(output.exit_code, Some(1));
        assert!(output.stderr.contains(&format!("Unknown option: {flag}")));
        assert!(!top.stdout.contains(flag));
    }
    let package = run(&rust, &["help", "package"], &root);
    assert!(package
        .stdout
        .contains("Packages can provide skills, prompts, and themes."));
    let update = run(&rust, &["help", "update"], &root);
    assert_eq!(update.exit_code, Some(0));
    assert!(update.stdout.contains("uninstalls the TypeScript version"));
    for flag in [
        "--check",
        "--archive <path>",
        "--source <url>",
        "--rollback",
        "--nightly",
        "--stable",
    ] {
        assert!(update.stdout.contains(flag), "missing {flag}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
