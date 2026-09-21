//! The SageDock Linux runtime: installing it from a local SageMath package, talking to it,
//! and the Windows-side workspace that deliberately outlives it.
//!
//! Layering is intentional. `wsl` is the only module that executes WSL commands, `image`
//! reads and verifies the SageMath package, `workspace` prepares Windows folders, and
//! `provision` composes them into the setup state machine. Coursework backup and folder
//! management live in the application layer, independently of runtime replacement.

pub mod health;
pub mod image;
pub mod provision;
pub mod repair;
pub mod workspace;
pub mod wsl;

pub use provision::{PersistedSetupState, SetupOutcome, SetupProgress};

/// Paths every SageDock runtime image guarantees. Built by `scripts/runtime/provision.sh`.
///
/// The app depends only on these, never on how the image was assembled inside (conda or
/// apt, which SageMath version, where Python lives), so a new image can change all of that
/// without requiring an app update.
pub mod contract {
    pub const RUNTIME_INFO: &str = "/opt/sagedock/runtime.json";
    pub const JUPYTER_LAUNCHER: &str = "/opt/sagedock/bin/sagedock-jupyter";
    pub const VERIFY: &str = "/opt/sagedock/bin/sagedock-verify";
    pub const SELFTEST: &str = "/opt/sagedock/bin/sagedock-selftest";
}
