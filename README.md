# Kapacitor UDF Library for Rust

## Overview

This library provides a Rust implementation for creating User-Defined Functions (UDFs) for Kapacitor, the native data processing engine for the InfluxDB time series database. It offers a high-performance, memory-efficient alternative to existing Python and Go implementations.

## Features

- Asynchronous I/O using async-std for efficient handling of data streams
- Support for both Unix socket and stdio communication
- Proper error handling and logging with the `tracing` crate
- Modular design for easy extension and customization
- Memory-efficient processing suitable for large-scale time series data

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
kapacitor-udf = "0.1.0"  # Replace with the actual version
```

## Quick Start

Here's a simple example of how to create a UDF handler and accepter:

```rust
use kapacitor_udf::{Handler, AccepterTrait, InfoResponse, Point, Server, StdioServer, SocketServer, // ... other imports};
use async_std::os::unix::net::UnixStream;
use async_trait::async_trait;

struct MyHandler;

#[async_trait]
impl Handler for MyHandler {
    async fn info(&self) -> Result<InfoResponse, std::io::Error> {
        // Implement info method
    }

    async fn init(&mut self, init: &InitRequest) -> Result<InitResponse, std::io::Error> {
        // Implement init method
    }

    async fn point(&mut self, point: &Point) -> Result<(), std::io::Error> {
        // Process each data point
    }

    // Implement other required methods...
}

#[derive(Debug)]
struct MyAccepter;

impl AccepterTrait for MyAccepter {
    fn accept(&self, conn: UnixStream) {
        // Handle the incoming connection
        async_std::task::spawn(async move {
            // Your connection handling logic here
        });
    }
}

#[async_std::main]
async fn main() -> std::io::Result<()> {
    // For Unix socket communication
    let handler = Box::new(MyHandler);
    let accepter = MyAccepter;
    let listener = async_std::os::unix::net::UnixListener::bind("/tmp/my_udf.sock").await?;
    let server = SocketServer::new(listener, accepter);
    server.serve().await?;

    // Or for stdio communication
    // let handler = Box::new(MyHandler);
    // let mut server = StdioServer::new(handler)?;
    // server.run().await
}
```

## Usage

1. Implement the `Handler` trait for your UDF logic.
2. If using Unix socket communication, implement the `AccepterTrait` for connection handling.
3. Choose between `SocketServer` (for Unix socket) or `StdioServer` (for stdio) based on your communication needs.
4. Start your UDF server using the appropriate server implementation.
5. Configure Kapacitor to use your Rust UDF (refer to InfluxDB documentation for this step).

## Performance

While comprehensive benchmarks are still in progress, initial tests show significant improvements in memory usage compared to Python implementations. Performance is expected to be on par with or better than Go implementations, especially for computationally intensive tasks.

## Current Status

This is an initial release, and there are known bugs that are being actively worked on, particularly with batch processing (see Known Issues section). The core functionality is in place, but users should expect some instability and potential API changes in future versions. Streaming operations have shown stability, but caution is advised for production use until a more thoroughly tested version is released.

## Known Issues

- **Batch Processing Bug**: There is an intermittent issue with batch processing where the `beginBatch` message isn't sent successfully. This doesn't occur every time, but users should be aware of potential inconsistencies in batch processing. We are actively investigating this issue.

- **Streaming Stability**: In contrast to the batch processing issue, streaming operations have been observed to be stable so far.

We are actively working on resolving these issues and welcome any feedback or contributions from the community to help improve the library's stability and performance.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the [MIT License](LICENSE).

## Acknowledgments

- The InfluxData team for creating Kapacitor and the UDF interface
- The Rust community for providing excellent async tools and libraries
