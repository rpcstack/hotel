//! Helpers shared by the services that talk to MongoDB.

/// Run a MongoDB operation to completion in its own task.
///
/// A handler future is dropped as soon as its client times out or resets the
/// stream (and a dropped `JoinSet` aborts its tasks). Abandoning a `mongodb`
/// 2.8.2 operation half-way through leaves the driver's connection unable to
/// complete later operations: after an overload every later request that
/// touches that client hangs (see the README, "Known issue"). Spawning the
/// call means a cancelled request only stops waiting for the result; the driver
/// operation itself always runs to completion. A panic inside the operation
/// surfaces as `Status::internal`.
pub async fn detached<T, F>(fut: F) -> Result<T, tonic::Status>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(fut)
        .await
        .map_err(|e| tonic::Status::internal(format!("mongodb task failed: {e}")))
}
