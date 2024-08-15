//! Server module for handling standard input/output connections in a Kapacitor UDF context.
//!
//! This module provides a `StdioServer` struct that manages communication via
//! standard input and output streams. It uses async-std for asynchronous I/O
//! and supports graceful shutdown.
//!
//! # Examples
//!
//! ```no_run
//! use your_crate::{StdioServer, YourHandlerImpl};
//! use async_std::task;
//!
//! #[async_std::main]
//! async fn main() -> std::io::Result<()> {
//!     let handler = YourHandlerImpl::new();
//!     let server = StdioServer::new(Box::new(handler))?;
//!
//!     // Start serving
//!     server.run().await
//! }
//! ```

use crate::agent::Agent;
use crate::traits::Handler;
use async_std::io::{stdin, stdout};
use async_std::task;
use async_trait::async_trait;
use std::io;
use tracing::{debug, error, info, instrument};

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
    fn new(handler: Box<dyn Handler>) -> io::Result<Self>
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

/// A server that manages a UDF agent communicating via stdin/stdout.
#[derive(Debug)]
pub struct StdioServer {
    agent: Agent,
}

#[async_trait]
impl Server for StdioServer {
    /// Creates a new StdioServer instance.
    ///
    /// # Arguments
    ///
    /// * `handler` - The UDF handler that will process requests.
    ///
    /// # Returns
    ///
    /// A Result containing the new StdioServer instance or an IO error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use your_crate::{StdioServer, Server, MyHandler};
    ///
    /// # fn main() -> std::io::Result<()> {
    /// let handler = MyHandler::new();
    /// let server = StdioServer::new(Box::new(handler))?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(handler))]
    fn new(handler: Box<dyn Handler>) -> io::Result<Self> {
        info!("Creating new StdioServer instance");
        let stdin = stdin();
        let stdout = stdout();

        let mut agent = Agent::new(Box::new(stdin), Box::new(stdout));
        agent.set_handler(Some(handler));

        Ok(StdioServer { agent })
    }

    /// Runs the server, processing UDF requests from stdin and writing responses to stdout.
    ///
    /// This method starts the agent and continues running until an error occurs
    /// or the input stream is closed.
    ///
    /// # Returns
    ///
    /// A Result indicating success or an IO error if the server fails to run.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use your_crate::{StdioServer, Server, MyHandler};
    /// use async_std::task;
    ///
    /// # async fn run() -> std::io::Result<()> {
    /// let handler = MyHandler::new();
    /// let mut server = StdioServer::new(Box::new(handler))?;
    ///
    /// // Run the server
    /// server.run().await
    /// # }
    /// ```
    #[instrument]
    async fn run(&mut self) -> io::Result<()> {
        info!("Starting UDF server");

        self.agent.start().unwrap();

        info!("UDF server running, processing requests");

        // Wait for the agent to complete
        if let Err(e) = self.agent.wait().await {
            error!("Error while running agent: {:?}", e);
            return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
        }

        info!("UDF server shutting down");
        Ok(())
    }
}

/// Runs the UDF server with the provided handler.
///
/// This function creates a new `StdioServer` instance with the given handler,
/// and runs it in the current async context.
///
/// # Arguments
///
/// * `handler` - The UDF handler that will process requests.
///
/// # Returns
///
/// A `Result` indicating success or an IO error if the server fails to run.
///
/// # Examples
///
/// ```no_run
/// use your_crate::{run_udf, MyHandler};
///
/// # async fn example() -> std::io::Result<()> {
/// let handler = MyHandler::new();
/// run_udf(handler).await
/// # }
/// ```
#[instrument(skip(handler))]
pub async fn run_udf<H: Handler + 'static>(handler: H) -> io::Result<()> {
    debug!("Setting up UDF server");
    let mut server = StdioServer::new(Box::new(handler))?;
    server.run().await
}

/// Runs the UDF server with the provided handler in a blocking manner.
///
/// This function is useful for running the UDF server in a synchronous context.
/// It blocks the current thread and runs the server to completion.
///
/// # Arguments
///
/// * `handler` - The UDF handler that will process requests.
///
/// # Returns
///
/// A `Result` indicating success or an IO error if the server fails to run.
///
/// # Examples
///
/// ```no_run
/// use your_crate::{run_udf_blocking, MyHandler};
///
/// # fn main() -> std::io::Result<()> {
/// let handler = MyHandler::new();
/// run_udf_blocking(handler)
/// # }
/// ```
#[instrument(skip(handler))]
pub fn run_udf_blocking<H: Handler + 'static>(handler: H) -> io::Result<()> {
    info!("Running UDF server in blocking mode");
    task::block_on(async { run_udf(handler).await })
}
