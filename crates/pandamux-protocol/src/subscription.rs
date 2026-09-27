use pandamux_core::{EnvironmentId, RunId, ThreadEventKind, ThreadId};
use serde::{Deserialize, Serialize};

/// Parameters for subscribing to a real-time event stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeParams {
    pub topic: String,
    #[serde(default)]
    pub environment_id: Option<EnvironmentId>,
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    #[serde(default)]
    pub run_id: Option<RunId>,
    #[serde(default)]
    pub since_seq: Option<u64>,
}

/// Server acknowledgement of an event subscription.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeResult {
    pub subscription_id: String,
    pub current_seq: u64,
}

/// Parameters for cancelling an active subscription.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeParams {
    pub subscription_id: String,
}

/// Wire envelope delivering a subscription event to the client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub subscription_id: String,
    pub environment_id: EnvironmentId,
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    #[serde(default)]
    pub run_id: Option<RunId>,
    pub seq: u64,
    pub at_ms: u64,
    pub kind: ThreadEventKind,
}
