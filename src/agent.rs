use crate::{
    io::{read_message, write_message},
    proto::{request, response, ErrorResponse, KeepaliveResponse, Request, Response},
    traits::Handler,
    EitherError,
};
use async_std::{
    channel::{self, Receiver, Sender},
    io::{self},
    sync::{Arc, Mutex},
    task,
};
use futures::{future::select, pin_mut};
use futures_util::FutureExt;
use std::{
    fmt,
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
};
use tracing::{debug, error, info, trace, warn};

/// Represents an agent that handles communication between Kapacitor and a UDF.
///
/// The `Agent` struct manages the I/O operations, message handling, and
/// coordination between different components of the UDF system.
pub struct Agent {
    /// Reader for incoming messages from Kapacitor.
    in_reader: Arc<Mutex<Box<dyn io::Read + Send + Unpin>>>,

    /// Writer for outgoing messages to Kapacitor.
    out_writer: Arc<Mutex<Box<dyn io::Write + Send + Unpin>>>,

    /// Channel sender for responses to be written back to Kapacitor.
    responses: Arc<Mutex<Sender<Response>>>,

    /// Channel receiver for responses to be written back to Kapacitor.
    in_responses: Receiver<Response>,

    /// Optional channel receiver for write errors.
    write_err_c: Option<Receiver<io::Error>>,

    /// Optional channel receiver for read errors.
    read_err_c: Option<Receiver<io::Error>>,

    /// The handler for processing incoming requests.
    handler: Option<Box<dyn Handler>>,

    /// Atomic boolean flag indicating whether the agent is running.
    is_running: Arc<AtomicBool>,
}

impl Agent {
    /// Creates a new `Agent` instance.
    ///
    /// # Arguments
    ///
    /// * `in_reader` - A boxed trait object implementing `io::Read` for incoming messages.
    /// * `out_writer` - A boxed trait object implementing `io::Write` for outgoing messages.
    ///
    /// # Returns
    ///
    /// A new `Agent` instance.
    pub fn new(
        in_reader: Box<dyn io::Read + Send + Unpin>,
        out_writer: Box<dyn io::Write + Send + Unpin>,
    ) -> Self {
        let (responses_tx, responses_rx) = channel::bounded(100);

        Self {
            in_reader: Arc::new(Mutex::new(in_reader)),
            out_writer: Arc::new(Mutex::new(out_writer)),
            responses: Arc::new(Mutex::new(responses_tx)),
            in_responses: responses_rx,
            write_err_c: None,
            read_err_c: None,
            handler: None,
            is_running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Creates a new `Agent` instance.
    ///
    /// # Arguments
    ///
    /// * `in_reader` - A boxed trait object implementing `io::Read` for incoming messages.
    /// * `out_writer` - A boxed trait object implementing `io::Write` for outgoing messages.
    ///
    /// # Returns
    ///
    /// A new `Agent` instance.
    pub fn set_handler(&mut self, handler: Option<Box<dyn Handler>>) {
        self.handler = handler;
    }

    /// Starts the agent, initializing read and write loops.
    ///
    /// This method sets up error channels, spawns tasks for reading from Kapacitor,
    /// writing responses back to Kapacitor, and forwarding responses between channels.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an IO error if initialization fails.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The handler is not set before starting the agent.
    /// - Any of the spawned tasks encounter an error during execution.
    pub fn start(&mut self) -> io::Result<()> {
        if self.handler.is_none() {
            error!("Handler not set on the agent before starting");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Must set a Handler on the agent before starting",
            ));
        }

        let (read_err_tx, read_err_rx) = channel::bounded(1);
        let (write_err_tx, write_err_rx) = channel::bounded(1);

        self.read_err_c = Some(read_err_rx);
        self.write_err_c = Some(write_err_rx);

        let handler = self.handler.take().unwrap();
        let in_reader = Arc::clone(&self.in_reader);
        let responses = Arc::clone(&self.responses);
        let in_responses = self.in_responses.clone();
        let is_running = Arc::clone(&self.is_running);

        // Spawn read loop
        task::spawn(async move {
            if let Err(err) =
                Self::read_loop(handler, in_reader, responses.clone(), is_running.clone()).await
            {
                error!("Read loop error: {:?}", err);
                let error_response = Response {
                    message: Some(response::Message::Error(ErrorResponse {
                        error: err.to_string(),
                    })),
                };
                if let Err(e) = responses.lock().await.send(error_response).await {
                    error!("Failed to send error response: {:?}", e);
                }
                let _ = read_err_tx.send(err).await;
            }
        });

        // Spawn write loop
        let write_out = Arc::clone(&self.out_writer);
        let write_is_running = Arc::clone(&self.is_running);
        task::spawn(async move {
            if let Err(err) = Self::write_loop(in_responses, write_out, write_is_running).await {
                error!("Write loop error: {:?}", err);
                let _ = write_err_tx.send(err).await;
            }
        });

        // Spawn forward responses loop
        let forward_responses_tx = Arc::clone(&self.responses);
        let forward_responses_rx = self.in_responses.clone();
        let forward_is_running = Arc::clone(&self.is_running);
        task::spawn(async move {
            Self::forward_responses(
                forward_responses_rx,
                forward_responses_tx,
                forward_is_running,
            )
            .await;
        });

        info!("Agent started successfully");
        Ok(())
    }

    /// Processes incoming messages from Kapacitor.
    ///
    /// This method runs in a loop, reading messages, processing them with the handler,
    /// and sending responses when necessary.
    ///
    /// # Arguments
    ///
    /// * `handler` - The handler for processing incoming requests.
    /// * `in_reader` - The input reader for incoming messages.
    /// * `responses` - The channel for sending responses.
    /// * `is_running` - An atomic flag indicating whether the loop should continue running.
    ///
    /// # Returns
    ///
    /// An `io::Result` indicating success or an IO error if reading or processing fails.
    async fn read_loop(
        mut handler: Box<dyn Handler>,
        in_reader: Arc<Mutex<Box<dyn io::Read + Send + Unpin>>>,
        responses: Arc<Mutex<Sender<Response>>>,
        is_running: Arc<AtomicBool>,
    ) -> io::Result<()> {
        let mut buf = Vec::with_capacity(4096); // Pre-allocate buffer
        let mut request = Request::default();

        while is_running.load(Ordering::Relaxed) {
            debug!("Waiting for next message from Kapacitor...");
            let mut reader = in_reader.lock().await;

            match read_message(&mut buf, &mut *reader, &mut request).await {
                Ok(()) => {
                    debug!("Received message: {:?}", request);
                    drop(reader); // Release the lock as soon as possible

                    if let Some(response) = Self::process_request(&mut handler, &request).await? {
                        debug!("Sending response: {:?}", response);
                        if let Err(e) = responses.lock().await.send(response).await {
                            error!("Failed to send response: {:?}", e);
                            break;
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    warn!("Reached end of stream (UnexpectedEof). Closing connection.");
                    break;
                }
                Err(e) => {
                    error!("Error reading message: {:?}", e);
                    return Err(e);
                }
            }
        }

        info!("Exiting message processing loop");
        handler.stop().await;
        debug!("Handler stopped");

        Ok(())
    }

    /// Processes a single request and generates a response if necessary.
    ///
    /// # Arguments
    ///
    /// * `handler` - The handler for processing the request.
    /// * `request` - The incoming request to process.
    ///
    /// # Returns
    ///
    /// A `Result` containing an `Option<Response>`. The `Option` is `Some` if a response
    /// should be sent back to Kapacitor, or `None` if no response is needed.
    async fn process_request(
        handler: &mut Box<dyn Handler>,
        request: &Request,
    ) -> io::Result<Option<Response>> {
        match &request.message {
            Some(request::Message::Info(_)) => {
                info!("Processing Info request");
                let info = handler.info().await?;
                debug!("Info response: {:?}", info);
                Ok(Some(Response {
                    message: Some(response::Message::Info(info)),
                }))
            }
            Some(request::Message::Init(init)) => {
                info!("Processing Init request: {:?}", init);
                let init_response = handler.init(init).await?;
                debug!("Init response: {:?}", init_response);
                Ok(Some(Response {
                    message: Some(response::Message::Init(init_response)),
                }))
            }
            Some(request::Message::Keepalive(keepalive)) => {
                trace!("Processing Keepalive request: {:?}", keepalive);
                Ok(Some(Response {
                    message: Some(response::Message::Keepalive(KeepaliveResponse {
                        time: keepalive.time,
                    })),
                }))
            }
            Some(request::Message::Snapshot(_)) => {
                info!("Processing Snapshot request");
                let snapshot = handler.snapshot().await?;
                debug!("Snapshot response: {:?}", snapshot);
                Ok(Some(Response {
                    message: Some(response::Message::Snapshot(snapshot)),
                }))
            }
            Some(request::Message::Restore(restore)) => {
                info!("Processing Restore request: {:?}", restore);
                let restore_response = handler.restore(restore).await?;
                debug!("Restore response: {:?}", restore_response);
                Ok(Some(Response {
                    message: Some(response::Message::Restore(restore_response)),
                }))
            }
            Some(request::Message::Begin(begin)) => {
                debug!("Processing Begin batch: {:?}", begin);
                handler.begin_batch(begin).await?;
                Ok(None)
            }
            Some(request::Message::Point(point)) => {
                trace!("Processing Point: {:?}", point);
                handler.point(point).await?;
                Ok(None)
            }
            Some(request::Message::End(end)) => {
                debug!("Processing End batch: {:?}", end);
                handler.end_batch(end).await?;
                Ok(None)
            }
            None => {
                warn!("Received message with no content");
                Ok(None)
            }
        }
    }

    /// Continuously writes responses back to Kapacitor.
    ///
    /// This method runs in a loop, receiving responses from a channel and writing
    /// them to the output stream.
    ///
    /// # Arguments
    ///
    /// * `resp` - The receiver channel for responses to be written.
    /// * `out` - The output writer for sending messages to Kapacitor.
    /// * `is_running` - An atomic flag indicating whether the loop should continue running.
    ///
    /// # Returns
    ///
    /// An `io::Result` indicating success or an IO error if writing fails.
    async fn write_loop(
        resp: Receiver<Response>,
        out: Arc<Mutex<Box<dyn io::Write + Send + Unpin>>>,
        is_running: Arc<AtomicBool>,
    ) -> io::Result<()> {
        debug!("Entering write loop");

        while is_running.load(Ordering::Relaxed) {
            match resp.recv().await {
                Ok(response) => {
                    debug!("Received response in write loop: {:?}", response);
                    let mut writer = out.lock().await;
                    write_message(&response, &mut *writer).await?;
                    debug!("Successfully wrote message");
                }
                Err(e) => {
                    warn!("Error receiving response, channel might be closed: {:?}", e);
                    break;
                }
            }
        }

        info!("Exiting write loop");
        Ok(())
    }

    /// Forwards responses between channels.
    ///
    /// This method runs in a loop, receiving responses from one channel and
    /// sending them to another. It's used to bridge the gap between the
    /// response generation and the write loop.
    ///
    /// # Arguments
    ///
    /// * `responses_rx` - The receiver channel for incoming responses.
    /// * `responses_tx` - The sender channel for outgoing responses.
    /// * `is_running` - An atomic flag indicating whether the loop should continue running.
    async fn forward_responses(
        responses_rx: Receiver<Response>,
        responses_tx: Arc<Mutex<Sender<Response>>>,
        is_running: Arc<AtomicBool>,
    ) {
        while is_running.load(Ordering::Relaxed) {
            match responses_rx.recv().await {
                Ok(response) => {
                    if let Err(e) = responses_tx.lock().await.send(response).await {
                        error!("Failed to forward response: {:?}", e);
                        break;
                    }
                }
                Err(e) => {
                    warn!(
                        "Error receiving response for forwarding or channel closed: {:?}",
                        e
                    );
                    break;
                }
            }
        }
        info!("Exiting forward_responses loop");
    }

    /// Waits for the agent to complete its operations and handles any errors.
    ///
    /// This method sets the `is_running` flag to false and then waits for both
    /// the read and write operations to complete, capturing any errors that occur.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an error if either the read or write operation failed.
    ///
    /// # Errors
    ///
    /// This method will return an error if either the read or write operation encounters an error.
    pub async fn wait(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.is_running.store(false, Ordering::SeqCst);

        let mut read_err = None;
        let mut write_err = None;

        while self.read_err_c.is_some() || self.write_err_c.is_some() {
            let read_fut = self.read_err_c.as_ref().map(|rx| rx.recv());
            let write_fut = self.write_err_c.as_ref().map(|rx| rx.recv());

            let result =
                match (read_fut, write_fut) {
                    (Some(r), Some(w)) => {
                        Self::race(
                            r.map(|res| match res {
                                Ok(err) => EitherError::Read(err),
                                Err(_) => EitherError::Read(io::Error::new(
                                    io::ErrorKind::Other,
                                    "channel closed",
                                )),
                            }),
                            w.map(|res| match res {
                                Ok(err) => EitherError::Write(err),
                                Err(_) => EitherError::Write(io::Error::new(
                                    io::ErrorKind::Other,
                                    "channel closed",
                                )),
                            }),
                        )
                        .await
                    }
                    (Some(r), None) => EitherError::Read(r.await.unwrap_or_else(|_| {
                        io::Error::new(io::ErrorKind::Other, "channel closed")
                    })),
                    (None, Some(w)) => EitherError::Write(w.await.unwrap_or_else(|_| {
                        io::Error::new(io::ErrorKind::Other, "channel closed")
                    })),
                    (None, None) => break,
                };

            match result {
                EitherError::Read(err) => {
                    self.read_err_c.take();
                    read_err = Some(err);
                }
                EitherError::Write(err) => {
                    self.write_err_c.take();
                    write_err = Some(err);
                }
            }
        }

        if let Some(err) = read_err {
            return Err(format!("read error: {}", err).into());
        }

        if let Some(err) = write_err {
            return Err(format!("write error: {}", err).into());
        }

        Ok(())
    }

    /// Races two futures to completion, returning the result of the first to finish.
    ///
    /// This method takes two futures and runs them concurrently, returning the
    /// result of whichever future completes first.
    ///
    /// # Type Parameters
    ///
    /// * `F1`: The type of the first future.
    /// * `F2`: The type of the second future.
    /// * `T`: The output type of both futures.
    ///
    /// # Arguments
    ///
    /// * `fut1`: The first future to race.
    /// * `fut2`: The second future to race.
    ///
    /// # Returns
    ///
    /// The result of whichever future completes first.
    async fn race<F1, F2, T>(fut1: F1, fut2: F2) -> T
    where
        F1: Future<Output = T> + Unpin,
        F2: Future<Output = T> + Unpin,
    {
        pin_mut!(fut1, fut2);
        select(fut1, fut2).await.factor_first().0
    }

    pub fn responses(&self) -> Arc<Mutex<Sender<Response>>> {
        self.responses.clone()
    }
}

impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Agent {{")?;
        writeln!(f, "    in_reader: <Box<dyn io::Read + Send + Unpin>>")?;
        writeln!(f, "    out_writer: <Box<dyn io::Write + Send + Unpin>>")?;
        writeln!(f, "    responses: <Arc<Mutex<Sender<Response>>>>")?;
        writeln!(f, "    in_responses: <Receiver<Response>>")?;
        writeln!(
            f,
            "    write_err_c: {}",
            if self.write_err_c.is_some() {
                "Some(<Receiver<io::Error>>)"
            } else {
                "None"
            }
        )?;
        writeln!(
            f,
            "    read_err_c: {}",
            if self.read_err_c.is_some() {
                "Some(<Receiver<io::Error>>)"
            } else {
                "None"
            }
        )?;
        writeln!(
            f,
            "    handler: {}",
            if self.handler.is_some() {
                "Some(<Box<dyn Handler>>)"
            } else {
                "None"
            }
        )?;
        writeln!(
            f,
            "    is_running: {}",
            self.is_running.load(std::sync::atomic::Ordering::Relaxed)
        )?;
        write!(f, "}}")
    }
}

impl fmt::Debug for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Agent")
            .field("in_reader", &"<Box<dyn io::Read + Send + Unpin>>")
            .field("out_writer", &"<Box<dyn io::Write + Send + Unpin>>")
            .field("responses", &"<Arc<Mutex<Sender<Response>>>>")
            .field("in_responses", &"<Receiver<Response>>")
            .field(
                "write_err_c",
                &self.write_err_c.as_ref().map(|_| "<Receiver<io::Error>>"),
            )
            .field(
                "read_err_c",
                &self.read_err_c.as_ref().map(|_| "<Receiver<io::Error>>"),
            )
            .field(
                "handler",
                &self.handler.as_ref().map(|_| "<Box<dyn Handler>>"),
            )
            .field(
                "is_running",
                &self.is_running.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish()
    }
}
