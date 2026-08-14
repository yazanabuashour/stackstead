mod contract;
mod destroy;
mod inspect;
mod inspect_health;
mod lease;
mod project;
mod provision;
mod provision_manifest;
mod provision_plan;
mod provision_source;
mod stop;
mod teardown;
mod types;
mod up;
mod up_database;
mod validation;

pub use contract::{install_dependencies, regenerate_contract, template_context};
pub use destroy::{destroy, resolve_destroy};
pub use inspect::inspect;
pub use lease::verify_port_leases;
pub use project::{
    compose_apply_with_file, compose_plan, compose_plan_with_file, current, init_with_compose_file,
    load_project,
};
pub use provision::{adopt, create, create_for_launch};
pub use stop::stop;
pub use teardown::ensure_no_teardown;
pub use types::{
    CreateOutcome, CurrentIdentity, EffectiveComponent, EffectiveStatus, InspectOutput, LiveStatus,
    ProjectRuntime, StatusBasis, UpOutcome, UpTimings,
};
pub use up::{up, up_after_create};
pub use validation::{
    validate_current_contract, validate_manifest_binding, validate_pointer_binding,
    validate_source_binding,
};

#[cfg(test)]
mod tests;
