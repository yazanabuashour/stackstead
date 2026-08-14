use super::{Cli, presentation::next_actions};
use clap::Parser as _;

use crate::{manifest::ComponentStatus, test_support::TestResultExt as _};

#[test]
fn json_is_global_after_subcommand() -> anyhow::Result<()> {
    assert!(
        Cli::try_parse_from(["stackstead", "ps", "--json"])
            .test()?
            .json,
        "--json was not treated as a global option after the subcommand"
    );
    Ok(())
}

#[test]
fn inspect_actions_use_the_full_id_and_runtime_state() -> anyhow::Result<()> {
    for (status, runtime_action) in [
        (
            ComponentStatus::Stopped,
            "stackstead up demo-feature-a-b123",
        ),
        (
            ComponentStatus::Running,
            "stackstead logs demo-feature-a-b123 --tail 200",
        ),
        (ComponentStatus::Unknown, "stackstead doctor"),
    ] {
        assert_eq!(
            next_actions("demo-feature-a-b123", status),
            [
                runtime_action,
                "stackstead context demo-feature-a-b123 --print",
            ],
            "next actions did not match the observed runtime state"
        );
    }
    Ok(())
}
