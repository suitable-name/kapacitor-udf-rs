use crate::proto::{
    BeginBatch, EndBatch, InfoResponse, InitRequest, InitResponse, Point, RestoreRequest,
    RestoreResponse, SnapshotResponse,
};
use async_std::os::unix::net::UnixStream;
use async_trait::async_trait;
use std::{fmt::Debug, io};

/// The `Handler` trait defines the interface for processing UDF requests.
///
/// Implementors of this trait are responsible for handling various types of
/// requests from Kapacitor, including initialization, data processing, and
/// state management.
#[async_trait]
pub trait Handler: Send + Sync {
    /// Provides information about the UDF.
    ///
    /// This method is called to retrieve metadata about the UDF, such as
    /// the types of data it can process and any options it supports.
    ///
    /// # Returns
    ///
    /// A `Result` containing an `InfoResponse` or an IO error.
    async fn info(&self) -> Result<InfoResponse, io::Error>;

    /// Initializes the UDF with the given configuration.
    ///
    /// This method is called when the UDF is first started, allowing it to
    /// set up its initial state based on the provided configuration.
    ///
    /// # Arguments
    ///
    /// * `init` - The initialization request containing configuration options.
    ///
    /// # Returns
    ///
    /// A `Result` containing an `InitResponse` or an IO error.
    async fn init(&mut self, init: &InitRequest) -> Result<InitResponse, io::Error>;

    /// Creates a snapshot of the UDF's current state.
    ///
    /// This method is used for creating backups or checkpoints of the UDF's state.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `SnapshotResponse` or an IO error.
    async fn snapshot(&self) -> Result<SnapshotResponse, io::Error>;

    /// Restores the UDF's state from a snapshot.
    ///
    /// This method is used to restore the UDF to a previous state, typically
    /// during recovery operations.
    ///
    /// # Arguments
    ///
    /// * `req` - The restore request containing the snapshot data.
    ///
    /// # Returns
    ///
    /// A `Result` containing a `RestoreResponse` or an IO error.
    async fn restore(&mut self, req: &RestoreRequest) -> Result<RestoreResponse, io::Error>;

    /// Signals the start of a batch of data points.
    ///
    /// This method is called at the beginning of a batch of data points,
    /// allowing the UDF to prepare for processing multiple points.
    ///
    /// # Arguments
    ///
    /// * `begin` - Information about the beginning of the batch.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an IO error.
    async fn begin_batch(&mut self, begin: &BeginBatch) -> Result<(), io::Error>;

    /// Processes a single data point.
    ///
    /// This method is called for each data point that needs to be processed by the UDF.
    ///
    /// # Arguments
    ///
    /// * `point` - The data point to process.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an IO error.
    async fn point(&mut self, point: &Point) -> Result<(), io::Error>;

    /// Signals the end of a batch of data points.
    ///
    /// This method is called at the end of a batch of data points,
    /// allowing the UDF to finalize any batch-level processing.
    ///
    /// # Arguments
    ///
    /// * `end` - Information about the end of the batch.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an IO error.
    async fn end_batch(&mut self, end: &EndBatch) -> Result<(), io::Error>;

    /// Stops the UDF and performs any necessary cleanup.
    ///
    /// This method is called when the UDF is being shut down.
    async fn stop(&mut self);
}

/// Trait for types that can accept Unix stream connections.
///
/// Implementors of this trait are responsible for handling new connections
/// accepted by the `Server`. This could involve spawning new tasks to handle
/// the connection, adding it to a connection pool, or any other custom logic.
pub trait AccepterTrait: Send + Sync + Debug {
    /// Accepts a new Unix stream connection.
    ///
    /// This method is called by the `Server` when a new connection is accepted.
    /// The implementation should handle the connection appropriately.
    ///
    /// # Arguments
    ///
    /// * `conn` - The accepted Unix stream connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_std::os::unix::net::UnixStream;
    /// use your_crate::Accepter;
    ///
    /// #[derive(Debug)]
    /// struct MyAccepter;
    ///
    /// impl Accepter for MyAccepter {
    ///     fn accept(&self, conn: UnixStream) {
    ///         // Handle the connection, e.g., spawn a new task
    ///         async_std::task::spawn(async move {
    ///             // Your connection handling logic here
    ///         });
    ///     }
    /// }
    /// ```
    fn accept(&self, conn: UnixStream);
}

/// Trait defining the interface for a UDF server.
#[async_trait]
pub trait Server {
    /// Creates a new instance of the server.
    ///
    /// # Arguments
    ///
    /// * `handler` - The UDF handler that will process requests.
    ///
    /// # Returns
    ///
    /// A Result containing the new server instance or an IO error.
    async fn new(handler: Box<dyn Handler>) -> io::Result<Self>
    where
        Self: Sized;

    /// Runs the server, processing UDF requests.
    ///
    /// This method should start the server and continue running until
    /// an error occurs or the input stream is closed.
    ///
    /// # Returns
    ///
    /// A Result indicating success or an IO error if the server fails to run.
    async fn run(&mut self) -> io::Result<()>;
}
