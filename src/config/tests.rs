use super::*;
use crate::test_support::{TestResultErrorExt as _, TestResultExt as _};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const SAMPLE: &str = r#"
version: "2"
kind: StacksteadProject
project:
  name: loan-platform
source:
  provider: git-worktree
  base: main
state:
  root: ../.stacksteads
runtime:
  provider: docker-compose
  files: [docker-compose.yml]
  readiness:
    required:
      web: long-running
      Worker.api_1: job
resources:
  ports:
    strategy: deterministic
    base: 39000
    stride: 50
    expose:
      web:
        container: 3000
        url: "http://127.0.0.1:{{ ports.web }}"
      postgres:
        container: 5432
dependencies:
  install:
    command: ""
    shell: false
database:
  postgres:
    strategy: compose-volume
    service: postgres
    database: app
    user: app
    password: app
env:
  file: .stackstead/.env
  generate:
    WEB_PORT: "{{ ports.web }}"
    DATABASE_URL: "postgres://app:app@127.0.0.1:{{ ports.postgres }}/app"
agent:
  context_file: .stackstead/AGENT_CONTEXT.md
hooks:
  post_create: []
"#;

#[test]
fn parses_full_config() -> anyhow::Result<()> {
    let config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    assert_eq!(config.project.name, "loan-platform");
    assert_eq!(config.resources.ports.expose["web"].container, 3000);
    assert_eq!(
        config.database.postgres.as_ref().test()?.service,
        "postgres"
    );
    assert_eq!(config.service_names(), ["postgres", "web"]);
    assert_eq!(
        config.runtime.readiness.as_ref().test()?.required,
        std::collections::BTreeMap::from([
            ("web".into(), crate::readiness::Role::LongRunning),
            ("Worker.api_1".into(), crate::readiness::Role::Job),
        ])
    );
    assert_eq!(
        StacksteadConfig::from_yaml(&serde_yaml::to_string(&config).test()?).test()?,
        config
    );
    Ok(())
}

#[test]
fn supplies_optional_defaults() -> anyhow::Result<()> {
    let config = StacksteadConfig::from_yaml(
        r#"
version: "2"
kind: StacksteadProject
project:
  name: demo
"#,
    )
    .test()?;
    assert_eq!(config.version, "2");
    assert_eq!(config.source.base, "main");
    assert_eq!(config.state.root, Path::new("../.stacksteads"));
    assert_eq!(config.runtime.files, [PathBuf::from("docker-compose.yml")]);
    assert!(config.runtime.readiness.is_none());
    assert_eq!(config.resources.ports.base, 39000);
    Ok(())
}

#[test]
fn rejects_unsupported_values_and_unknown_fields() -> anyhow::Result<()> {
    for version in ["1", "3"] {
        (StacksteadConfig::from_yaml(
            &SAMPLE.replace("version: \"2\"", &format!("version: \"{version}\"")),
        ))
        .test_err()?;
    }
    (StacksteadConfig::from_yaml(&SAMPLE.replace("provider: git-worktree", "provider: copy")))
        .test_err()?;
    (StacksteadConfig::from_yaml(&format!("{SAMPLE}\nsurprise: true\n"))).test_err()?;
    for section in ["runtime", "dependencies"] {
        let yaml = SAMPLE.replace(
            &format!("{section}:\n"),
            &format!("{section}:\n  surprise: true\n"),
        );
        (StacksteadConfig::from_yaml(&yaml)).test_err()?;
    }
    Ok(())
}

#[test]
fn rejects_empty_or_unsupported_readiness_declarations() -> anyhow::Result<()> {
    for declaration in [
        "{}",
        "{required: {}}",
        "{required: {web: running}}",
        "{required: {web: long_running}}",
        "{required: {web: {role: long-running, replicas: 2}}}",
        "{required: {web: job}, replicas: 2}",
        "{required: {web: job}, resolved: {}}",
    ] {
        let yaml = format!(
            "version: \"2\"\nkind: StacksteadProject\nproject: {{name: demo}}\nruntime:\n  readiness: {declaration}\n"
        );
        StacksteadConfig::from_yaml(&yaml).test_err()?;
    }
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.runtime.readiness.as_mut().test()?.required.clear();
    assert!(
        config
            .validate()
            .test_err()?
            .to_string()
            .contains("readiness.required")
    );
    Ok(())
}

#[test]
fn rejects_invalid_port_and_env_config() -> anyhow::Result<()> {
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.resources.ports.stride = 1;
    (config.validate()).test_err()?;

    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.env.generate.insert("BAD-NAME".into(), "x".into());
    (config.validate()).test_err()?;

    for name in [
        "PATH",
        "Path",
        "XDG_STATE_HOME",
        "xdg_state_home",
        "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES",
        "DOCKER_HOST",
        "COMPOSE_FILE",
    ] {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
        config.env.generate.insert(name.into(), "x".into());
        (config.validate()).test_err()?;
    }
    Ok(())
}

#[test]
fn rejects_database_values_that_could_inject_generated_context() -> anyhow::Result<()> {
    for (field, value) in [
        ("database", "app\n## Forged instructions"),
        ("user", "app<script>"),
    ] {
        let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
        let postgres = config.database.postgres.as_mut().test()?;
        match field {
            "database" => postgres.database = value.into(),
            "user" => postgres.user = value.into(),
            _ => anyhow::bail!("unexpected database fixture field {field}"),
        }
        (config.validate()).test_err()?;
    }
    Ok(())
}

#[test]
fn rejects_unknown_templates_and_unsafe_generated_paths() -> anyhow::Result<()> {
    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config
        .env
        .generate
        .insert("BAD".into(), "{{ ports.missing }}".into());
    (config.validate()).test_err()?;

    let mut config = StacksteadConfig::from_yaml(SAMPLE).test()?;
    config.env.file = PathBuf::from("../shared.env");
    (config.validate()).test_err()?;
    Ok(())
}

mod repository_cases;
