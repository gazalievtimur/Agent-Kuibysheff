//! Wire format for parent ↔ helper re-exec protocol.

use serde::{Deserialize, Serialize};

use crate::request::SandboxLaunchRequest;

pub(crate) const HELPER_ENV: &str = "AGENT_KUIBYSHEV_LINUX_SANDBOX_HELPER";
pub(crate) const REQUEST_ENV: &str = "AGENT_KUIBYSHEV_LINUX_SANDBOX_REQUEST";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HelperRequest {
    pub launch: SandboxLaunchRequest,
}

impl HelperRequest {
    pub fn from_launch(launch: &SandboxLaunchRequest) -> Self {
        Self {
            launch: launch.clone(),
        }
    }
}
