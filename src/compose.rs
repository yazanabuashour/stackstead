mod apply;
mod claim;
mod contract;
mod docker;
mod model;
mod observations;
mod ownership;
mod ownership_model;
mod planning;
mod ports;
mod requirements;
mod resource_config;
mod resources;
mod runtime;
mod runtime_names;
mod services;
mod yaml;

pub use apply::apply_at;
pub use claim::{prepare_owned_source_removal, remove_runtime_claim, verify_owned_runtime};
pub use contract::{resolve_port_target, validate_port_contract};
pub use docker::{base_args, docker_environment};
pub use model::{ComposeApplyOutput, ComposePlan, ComposePortTarget, ServiceObservation};
pub use ownership::{
    ensure_service_configured, verify_ownership_override, write_ownership_override,
};
pub use planning::plan_at;
pub use ports::{all_interface_ports_in_file, fixed_ports_in_file, unbound_ports_in_file};
pub use requirements::resolve_requirements;
pub use runtime::{down_volumes, stop, up};
pub use services::{
    endpoint_is_published, ensure_endpoint_published, follow_logs, is_running, logs,
    postgres_is_ready, service_is_running, service_observations,
};

#[cfg(test)]
pub use apply::apply;
#[cfg(test)]
pub use model::ComposePortPlan;
#[cfg(test)]
pub use planning::{plan, plan_file};
#[cfg(test)]
pub use ports::detect_fixed_host_ports;

#[cfg(test)]
use claim::ownership_bind_mount;
#[cfg(test)]
use model::RUNTIME_TOKEN_LABEL;
#[cfg(test)]
use ownership::render_ownership_override;
#[cfg(test)]
use runtime_names::expected_runtime_names;
#[cfg(test)]
use services::{endpoint_matches, running_service_output, service_running_args};
#[cfg(test)]
use yaml::{port_declarations, yaml_field};

#[cfg(test)]
mod tests;
