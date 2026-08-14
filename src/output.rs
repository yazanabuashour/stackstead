use serde::Serialize;

const VERSION: &str = "1";
const INSPECTION_VERSION: &str = "3";

mod compose;
mod contract;
mod inspection;
mod summary;
mod utility;

pub use compose::{ComposeApplyOutput, ComposePlanOutput, PathOutput};
pub use inspection::StacksteadInspectionOutput;
pub use summary::{
    StacksteadChangeOutput, StacksteadCurrentOutput, StacksteadListOutput, StacksteadSummaryOutput,
};
pub use utility::{
    ContextOutput, DatabaseStatusOutput, DoctorOutput, EnvironmentOutput, LogsOutput, OpenOutput,
};

mod private {
    pub trait Sealed {}
}

pub trait CliOutput: private::Sealed + Serialize {}

macro_rules! cli_output {
    ($($type:ty),+ $(,)?) => {$(
        impl private::Sealed for $type {}
        impl CliOutput for $type {}
    )+};
}

cli_output!(
    PathOutput,
    ComposePlanOutput,
    ComposeApplyOutput,
    StacksteadChangeOutput,
    StacksteadListOutput,
    StacksteadCurrentOutput,
    StacksteadInspectionOutput,
    EnvironmentOutput,
    ContextOutput,
    LogsOutput,
    OpenOutput,
    DatabaseStatusOutput,
    DoctorOutput,
);
