//! Non-interactive informational output must not contaminate protocol stdout.

use std::process::Command;

#[test]
fn headless_information_uses_stderr_but_public_help_uses_stdout() {
    let sandbox = tempfile::tempdir().unwrap();
    let package = sandbox.path().join("package");
    std::fs::create_dir(&package).unwrap();
    std::fs::write(package.join("package.json"), r#"{"version":"1.2.3"}"#).unwrap();
    for (args, stderr_prefix) in [
        (vec!["--version"], "1.2.3\n"),
        (vec!["-v"], "1.2.3\n"),
        (vec!["--help"], "prime-agent - "),
        (vec!["-h"], "prime-agent - "),
        (vec!["model", "list"], "No models available."),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_prime-agent"))
            .args(&args)
            .env_clear()
            .env("HOME", sandbox.path())
            .env("USERPROFILE", sandbox.path())
            .env("PRIME_AGENT_CODING_AGENT_DIR", sandbox.path().join("agent"))
            .env("PI_PACKAGE_DIR", &package)
            .env("PI_OFFLINE", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .starts_with(stderr_prefix));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_prime-agent"))
        .arg("help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .starts_with("prime-agent - "));
}
