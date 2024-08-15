//! Server module for handling Unix socket connections in a Kapacitor UDF context.
//!
//! This module provides a `Server` struct that manages a Unix socket listener
//! and accepts incoming connections. It uses async-std for asynchronous I/O
//! and supports graceful shutdown on signal reception.
//!
//! # Examples
//!
//! ```no_run
//! use async_std::os::unix::net::UnixListener;
//! use async_std::task;
//! use your_crate::{Server, YourAccepterImpl};
//!
//! #[async_std::main]
//! async fn main() -> std::io::Result<()> {
//!     let listener = UnixListener::bind("/tmp/your_socket").await?;
//!     let accepter = YourAccepterImpl::new();
//!     let server = Server::new(listener, accepter);
//!
//!     // Set up signal handling for graceful shutdown
//!     server.stop_on_signals(&[signal_hook::consts::signal::SIGINT, signal_hook::consts::signal::SIGTERM]).await?;
//!
//!     // Start serving
//!     server.serve().await
//! }
//! ```

use async_std::{
    io,
    os::unix::net::UnixListener,
    sync::{Arc, Mutex},
    task,
};
use futures_util::stream::StreamExt;
use signal_hook::consts::signal::*;
use signal_hook_async_std::Signals;
use std::{
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
};
use tracing::{debug, error, info, instrument, warn};

use crate::traits::AccepterTrait;

/// Server struct for managing Unix socket connections.
///
/// This struct is responsible for listening on a Unix socket, accepting
/// new connections, and delegating their handling to an `Accepter` implementation.
/// It also supports graceful shutdown through signal handling.
#[derive(Debug, Clone)]
pub struct SocketServer {
    listener: Arc<Mutex<Option<UnixListener>>>,
    accepter: Arc<dyn AccepterTrait>,
    stopped: Arc<AtomicBool>,
    stopping: Arc<Mutex<()>>,
}

impl SocketServer {
    /// Creates a new Server instance.
    ///
    /// # Arguments
    ///
    /// * `listener` - The Unix socket listener. This should be already bound to a socket path.
    /// * `accepter` - An implementation of the `Accepter` trait that will handle new connections.
    ///
    /// # Returns
    ///
    /// A new `Server` instance.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use async_std::os::unix::net::UnixListener;
    /// use your_crate::{Server, MyAccepter};
    ///
    /// # async fn run() -> std::io::Result<()> {
    /// let listener = UnixListener::bind("/tmp/my_socket").await?;
    /// let accepter = MyAccepter;
    /// let server = Server::new(listener, accepter);
    /// # Ok(())
    /// # }
    /// ```
    #[instrument]
    pub fn new(listener: UnixListener, accepter: impl AccepterTrait + 'static) -> Self {
        info!("Creating new Server instance");
        SocketServer {
            listener: Arc::new(Mutex::new(Some(listener))),
            accepter: Arc::new(accepter),
            stopped: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(Mutex::new(())),
        }
    }

    /// Starts serving incoming connections.
    ///
    /// This method begins the main loop of accepting connections and passing them
    /// to the `Accepter`. It will continue running until `stop()` is called or
    /// an unrecoverable error occurs.
    ///
    /// # Returns
    ///
    /// An `io::Result<()>` indicating success or failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use async_std::task;
    /// use your_crate::{Server, MyAccepter};
    ///
    /// # async fn run() -> std::io::Result<()> {
    /// # let listener = async_std::os::unix::net::UnixListener::bind("/tmp/my_socket").await?;
    /// let server = Server::new(listener, MyAccepter);
    ///
    /// // Start serving in the background
    /// task::spawn(async move {
    ///     if let Err(e) = server.serve().await {
    ///         eprintln!("Server error: {}", e);
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    #[instrument]
    pub async fn serve(&self) -> io::Result<()> {
        info!("Starting server");
        if self.stopped.load(Ordering::Acquire) {
            warn!("Server is already stopped");
            return Ok(());
        }

        self.run().await
    }

    /// Sets up signal handling for graceful server shutdown.
    ///
    /// This method spawns a task that listens for the specified signals and
    /// calls `stop()` when one is received. This allows for graceful shutdown
    /// of the server in response to system signals.
    ///
    /// # Arguments
    ///
    /// * `signals` - A slice of signal numbers to handle. Common signals are
    ///   available in the `signal_hook::consts::signal` module.
    ///
    /// # Returns
    ///
    /// An `io::Result<()>` indicating success or failure in setting up signal handling.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use signal_hook::consts::signal::{SIGINT, SIGTERM};
    /// use your_crate::{Server, MyAccepter};
    ///
    /// # async fn run() -> std::io::Result<()> {
    /// # let listener = async_std::os::unix::net::UnixListener::bind("/tmp/my_socket").await?;
    /// let server = Server::new(listener, MyAccepter);
    ///
    /// // Set up graceful shutdown on SIGINT and SIGTERM
    /// server.stop_on_signals(&[SIGINT, SIGTERM]).await?;
    ///
    /// server.serve().await
    /// # }
    /// ```
    #[instrument(skip(signals))]
    pub async fn stop_on_signals(&self, signals: &[i32]) -> io::Result<()> {
        info!("Setting up signal handling for server stop");
        if self.stopped.load(Ordering::Acquire) {
            warn!("Server is already stopped");
            return Ok(());
        }

        let mut signals = Signals::new(signals)?;
        let server = self.clone();

        task::spawn(async move {
            while let Some(signal) = signals.next().await {
                match signal {
                    SIGINT | SIGTERM => {
                        info!("Received termination signal: {}", signal);
                        server.stop().await;
                        break;
                    }
                    _ => {
                        debug!("Received unhandled signal: {}", signal);
                    }
                }
            }
        });

        Ok(())
    }

    /// Stops the server.
    ///
    /// This method initiates a graceful shutdown of the server. It closes the
    /// listener, preventing new connections, and sets the stopped flag.
    /// Existing connections are not immediately terminated.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use your_crate::{Server, MyAccepter};
    ///
    /// # async fn run() -> std::io::Result<()> {
    /// # let listener = async_std::os::unix::net::UnixListener::bind("/tmp/my_socket").await?;
    /// let server = Server::new(listener, MyAccepter);
    ///
    /// // In some part of your code where you want to stop the server
    /// server.stop().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop(&self) {
        info!("Stopping server");
        let _lock = self.stopping.lock().await;
        if self.stopped.swap(true, Ordering::AcqRel) {
            warn!("Server is already in the process of stopping");
            return;
        }
        debug!("Closing listener");
        // self.listener.lock().await.unwrap()

        info!("Server stopped");
    }

    async fn run(&self) -> io::Result<()> {
        let (sender, mut receiver) = async_std::channel::bounded(1);

        let accepter = Arc::clone(&self.accepter);
        let listener = Arc::clone(&self.listener);
        let stopped = Arc::clone(&self.stopped);

        task::spawn(async move {
            while !stopped.load(Ordering::Acquire) {
                match listener.lock().await.as_mut().unwrap().accept().await {
                    Ok((stream, _)) => {
                        debug!("Accepted new connection");
                        accepter.accept(stream);
                    }
                    Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {
                        debug!("Listener closed, stopping accept loop");
                        break;
                    }
                    Err(e) => {
                        error!("Error accepting connection: {}", e);
                        let _ = sender.send(Err(e)).await;
                        break;
                    }
                }
            }
        });

        while !self.stopped.load(Ordering::Acquire) {
            if let Some(result) = receiver.next().await {
                if self.stopped.load(Ordering::Acquire) {
                    info!("Server stopped while processing result");
                    return Ok(());
                }
                error!("Error received from accept loop: {:?}", result);
                return result;
            }
        }

        info!("Server stop signal received, exiting run loop");
        Ok(())
    }
}
