//! A Rust implementation of Kapacitor User-Defined Functions (UDFs).
//!
//! This crate provides a framework for creating and managing UDFs that can be
//! integrated with Kapacitor, a real-time streaming data processing engine.
//! It includes functionality for handling communication protocols, managing
//! data flow, and implementing custom data processing logic.
//!
//! # Modules
//!
//! - `proto`: Contains the generated Protocol Buffers code for communication with Kapacitor.
//! - `agent`: Implements the `Agent` struct, which manages the lifecycle and communication of a UDF.
//! - `io`: Provides utilities for reading and writing Protocol Buffer messages.
//! - `server`: Implements the server-side logic for handling UDF connections.
//! - `traits`: Defines the `Handler` trait, which users implement to create custom UDFs.
//!
//! # Examples
//!
//! To create a custom UDF, implement the `Handler` trait and use the `Agent` to manage its lifecycle:
//!
//! ```rust
//! use my_udf_crate::{traits::Handler, agent::Agent, proto};
//! use async_trait::async_trait;
//! use std::io;
//!
//! struct MyUDF;
//!
//! #[async_trait]
//! impl Handler for MyUDF {
//!     // Implement Handler methods...
//! }
//!
//! # fn main() -> io::Result<()> {
//! # async_std::task::block_on(async {
//! let mut agent = Agent::new(/* provide input and output */);
//! agent.set_handler(Some(Box::new(MyUDF)));
//! agent.start().await?;
//! # Ok(())
//! # })
//! # }
//! ```

// Re-export the generated protobuf code
pub mod proto {
    //! Generated Protocol Buffers code for Kapacitor UDF communication.
    //!
    //! This module is automatically generated from the `.proto` files defining
    //! the communication protocol between Kapacitor and UDFs.

    use tonic::include_proto;

    include_proto!("agent");
}

pub mod agent;
pub mod io;
pub mod stdio_server;
pub mod traits;
pub mod socket_server;

/// An enum representing either a read or write error.
///
/// This is used internally by the `Agent` to handle errors from
/// concurrent read and write operations.
#[derive(Debug)]
pub(crate) enum EitherError<T> {
    /// Represents an error that occurred during a read operation.
    Read(T),
    /// Represents an error that occurred during a write operation.
    Write(T),
}
