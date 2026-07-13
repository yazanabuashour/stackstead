pub const VERSION: u64 = 1;
pub const MARKER_PREFIX: &str = "<!-- stackstead-policy:";
pub const MARKER_SUFFIX: &str = "-->";
pub const FILE_NAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
pub const GUIDE_URL: &str =
    "https://github.com/yazanabuashour/stackstead/blob/main/docs/agent-setup.md#repository-policy";

pub fn marker() -> String {
    format!("{MARKER_PREFIX} {VERSION} {MARKER_SUFFIX}")
}

pub const TEXT: &str = r#"## Stackstead

For tasks that need services, ports, URLs, databases, migrations, or runtime
tests, work in a Stackstead—not the canonical checkout—and use Stackstead lifecycle
commands instead of bare Docker Compose.

If `$STACKSTEAD_CONTEXT` is set, read it, stay in `$STACKSTEAD_WORKTREE`, and use
only the ports, URLs, and database it provides. Otherwise, create a new environment
with `stackstead --json create <name>`, capture its full `stackstead_id`, run
`stackstead up <full-id>`, then enter it with
`stackstead run <full-id> -- <agent-or-command>`. Reuse an environment only when the user
or manager supplies its exact full ID."#;
