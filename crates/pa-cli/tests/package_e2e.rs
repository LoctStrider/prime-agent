//! Binary-level package install/remove persistence and scope selection.

use std::path::Path;
use std::process::{Command, Output};

fn run(base: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prime-agent"))
        .args(args)
        .env("HOME", base.join("home"))
        .env("PRIME_AGENT_CODING_AGENT_DIR", base.join("agent"))
        .current_dir(base.join("cwd"))
        .output()
        .expect("run package command")
}

#[test]
fn local_package_commands_persist_in_the_selected_scope() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    for path in ["home", "agent", "cwd/local-pkg"] {
        std::fs::create_dir_all(base.join(path)).unwrap();
    }
    let package = base.join("cwd/local-pkg");
    std::fs::write(package.join("package.json"), r#"{"name":"local-pkg"}"#).unwrap();

    for args in [
        vec!["package", "install", "./local-pkg"],
        vec!["package", "install", "./local-pkg", "--local"],
        vec!["package", "install", "./local-pkg", "--local"],
        vec!["package", "remove", "./local-pkg"],
    ] {
        let output = run(base, &args);
        assert!(output.status.success(), "{args:?}: {output:?}");
    }

    let settings = |path| {
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(base.join(path)).unwrap(),
        )
        .unwrap()
    };
    assert_eq!(
        settings("agent/settings.json"),
        serde_json::json!({"packages": []})
    );
    assert_eq!(
        settings("cwd/.prime/agent/settings.json"),
        serde_json::json!({"packages": ["../../local-pkg"]})
    );

    let output = run(base, &["package", "list"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "Project packages:\n  ../../local-pkg\n    {}\n",
            package.display()
        )
    );

    let output = run(base, &["package", "remove", "./local-pkg", "--local"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        settings("cwd/.prime/agent/settings.json"),
        serde_json::json!({"packages": []})
    );
    assert!(
        package.join("package.json").is_file(),
        "removing a local source keeps its files"
    );
}
