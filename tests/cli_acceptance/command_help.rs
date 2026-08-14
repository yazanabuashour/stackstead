use super::*;

#[test]
fn help_exposes_the_complete_command_surface() -> anyhow::Result<()> {
    let directory = tempfile::tempdir().test_context("create command directory")?;
    let assert = stackstead(directory.path())
        .arg("--help")
        .assert()
        .success();
    let help = String::from_utf8_lossy(&assert.get_output().stdout);
    for command in [
        "init", "compose", "create", "adopt", "up", "run", "exec", "launch", "ps", "current",
        "inspect", "env", "logs", "context", "open", "db", "stop", "destroy", "doctor", "repair",
    ] {
        assert!(help.contains(command), "top-level help omits {command:?}");
    }

    for args in [
        vec!["init", "--help"],
        vec!["compose", "plan", "--help"],
        vec!["compose", "apply", "--help"],
        vec!["create", "--help"],
        vec!["adopt", "--help"],
        vec!["up", "--help"],
        vec!["run", "--help"],
        vec!["exec", "--help"],
        vec!["launch", "--help"],
        vec!["ps", "--help"],
        vec!["current", "--help"],
        vec!["inspect", "--help"],
        vec!["env", "--help"],
        vec!["logs", "--help"],
        vec!["context", "--help"],
        vec!["open", "--help"],
        vec!["db", "status", "--help"],
        vec!["stop", "--help"],
        vec!["destroy", "--help"],
        vec!["doctor", "--help"],
        vec!["repair", "--help"],
    ] {
        stackstead(directory.path()).args(args).assert().success();
    }

    stackstead(directory.path())
        .args(["env", "demo", "--show-secrets"])
        .assert()
        .failure();
    Ok(())
}
