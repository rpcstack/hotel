//! The root context of a request: which API it is, its SLO and the absolute
//! deadline it must meet. This is application configuration, owned by whoever
//! creates the request (the load generator, the HTTP frontend), not by a
//! policy: `attach` stamps the deadline on the request as the standard gRPC
//! `grpc-timeout` header, which any server hook (and Tonic's own server
//! timeout) can read.
//!
//! The rpcstack framework core has no notion of deadlines or SLOs, so a policy
//! that wants budgets or deadline propagation across hops has to bring its own
//! module for that; the application only supplies this root deadline.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::timing::time_now;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// What the request traces record about a request.
#[derive(Debug, Clone)]
pub struct Context {
    api: String,
    request_id: u64,
    slo: u64,
    gateway_entry: u64,
    deadline: u64,
}

impl Context {
    pub fn api(&self) -> &str {
        &self.api
    }
    pub fn request_id(&self) -> u64 {
        self.request_id
    }
    /// The SLO in microseconds.
    pub fn slo(&self) -> u64 {
        self.slo
    }
    /// Wall-clock microseconds since the epoch at which the request entered.
    pub fn gateway_entry(&self) -> u64 {
        self.gateway_entry
    }
    /// `gateway_entry + slo`, in microseconds since the epoch.
    pub fn deadline(&self) -> u64 {
        self.deadline
    }
}

/// The context of a request entering the system now.
#[derive(Debug, Clone)]
pub struct RootContext(Context);

impl RootContext {
    pub fn new(api: &str, slo: Duration) -> Self {
        let slo = slo.as_micros() as u64;
        let gateway_entry = time_now();
        Self(Context {
            api: api.to_string(),
            request_id: NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
            slo,
            gateway_entry,
            deadline: gateway_entry + slo,
        })
    }

    /// Stamp the SLO on `request` as its `grpc-timeout`.
    pub fn attach<T>(&self, mut request: tonic::Request<T>) -> tonic::Request<T> {
        request.set_timeout(Duration::from_micros(self.0.slo.max(1)));
        request
    }

    pub fn context(&self) -> &Context {
        &self.0
    }

    pub fn into_context(self) -> Context {
        self.0
    }
}
