/// Why the sandbox Canvas could not start. Startup-only; never shown to callers.
#[derive(Debug, thiserror::Error)]
pub enum CanvasError {
    #[error("synthetic Canvas seed is invalid: {0}")]
    Seed(String),
    #[error("canvas.base_url must be an http URL on a loopback address")]
    NotLoopback,
    #[error("cannot bind the synthetic Canvas fake: {0}")]
    Bind(#[source] std::io::Error),
    #[error("the synthetic Canvas fake must start inside a Tokio runtime")]
    NoRuntime,
    #[error("cannot build the synthetic Canvas client")]
    Client,
}
