use super::*;

#[test]
fn init_writes_a_valid_config_and_refuses_to_overwrite_it() -> anyhow::Result<()> {
    let project = Project::git_repo()?;
    let config_path = project.repo.join("stackstead.yaml");
    let assert = stackstead(&project.repo)
        .args(["init", "--json"])
        .assert()
        .success();
    let initialized: Value = serde_json::from_slice(&assert.get_output().stdout).test()?;
    assert_eq!(
        initialized["kind"], "StacksteadInit",
        "test contract values differ"
    );
    assert_eq!(initialized["version"], "1", "test contract values differ");
    assert_eq!(
        initialized["path"],
        config_path.to_string_lossy().as_ref(),
        "test contract values differ"
    );

    let original = fs::read(&config_path).test_context("read initialized config")?;
    let config = load_config(&config_path)?;
    assert_eq!(config["version"], "1", "test contract values differ");
    assert_eq!(
        config["kind"], "StacksteadProject",
        "test contract values differ"
    );
    assert_eq!(
        config["project"]["name"], "demo-project",
        "test contract values differ"
    );
    assert_eq!(
        config["source"]["base"], "main",
        "test contract values differ"
    );

    let assert = stackstead(&project.repo).arg("init").assert().failure();
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr).contains("refusing to overwrite"),
        "test contract condition failed"
    );
    assert_eq!(
        fs::read(&config_path).test_context("reread initialized config")?,
        original,
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn init_records_the_exact_commit_for_a_detached_head() -> anyhow::Result<()> {
    let project = Project::git_repo()?;
    let head = git(&project.repo, &["rev-parse", "HEAD"])?;
    git(&project.repo, &["checkout", "--detach"])?;

    stackstead(&project.repo).arg("init").assert().success();

    let config = load_config(&project.repo.join("stackstead.yaml"))?;
    assert_eq!(
        config["source"]["base"],
        head.trim(),
        "test contract values differ"
    );
    Ok(())
}

#[test]
fn human_init_recommends_but_does_not_edit_repository_instructions() -> anyhow::Result<()> {
    let project = Project::git_repo()?;
    let instructions = project.repo.join("AGENTS.md");
    fs::write(&instructions, "# Human-owned policy\n").test()?;

    let assert = stackstead(&project.repo).arg("init").assert().success();
    let stdout = output_text(&assert.get_output().stdout)?;
    for expected in [
        "review, add, and commit this policy",
        "commands instead of bare Docker Compose",
        "stackstead --json create <name>",
        "stackstead up <full-id>",
        "stackstead run <full-id> -- <agent-or-command>",
        "only the ports, URLs, and database it provides",
        "Reuse an environment only when the user",
        "manager supplies its exact full ID",
        "<!-- stackstead-policy: 1 -->",
        "Stackstead may read recognized root instruction files during `doctor`",
        "does not edit human-owned agent instructions",
    ] {
        assert!(
            stdout.contains(expected),
            "init output omitted {expected:?}"
        );
    }
    assert!(
        !stdout.contains("stackstead --json ps"),
        "test contract condition failed"
    );
    assert_eq!(
        fs::read_to_string(&instructions).test()?,
        "# Human-owned policy\n",
        "test contract values differ"
    );

    assert!(
        !project.repo.join("CLAUDE.md").exists(),
        "test contract condition failed"
    );
    Ok(())
}
