//! I/O utilities for handling protocol buffer messages over streams.
//!
//! This module provides functionality for reading and writing protocol buffer messages
//! using varint-prefixed length encoding. It includes traits and functions for reading
//! individual bytes, exact byte counts, and complete messages, as well as writing
//! messages to streams.
//!
//! The main components are:
//! - `ByteReadReader`: A trait for reading individual bytes and exact byte counts.
//! - `read_message`: A function for reading a complete protocol buffer message.
//! - `write_message`: A function for writing a complete protocol buffer message.
//!
//! This module is designed to work with asynchronous I/O operations and is compatible
//! with the `async-std` runtime.

use async_std::io::{self, Read, ReadExt, Write, WriteExt};
use async_trait::async_trait;
use prost::Message;
use std::pin::Pin;
use tracing::{debug, error, instrument, trace, warn};

/// A trait for reading individual bytes and exact byte counts.
///
/// This trait is implemented for `Pin<Box<dyn Read + Send>>`, allowing it to be
/// used with various asynchronous I/O types.
#[async_trait]
pub trait ByteReadReader: Send {
    /// Reads a single byte from the underlying stream.
    ///
    /// # Returns
    ///
    /// A `Result` containing the read byte or an I/O error.
    async fn read_byte(&mut self) -> io::Result<u8>;

    /// Reads an exact number of bytes from the underlying stream.
    ///
    /// # Arguments
    ///
    /// * `buf` - A mutable slice to read the bytes into.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or an I/O error.
    async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()>;
}

#[async_trait]
impl ByteReadReader for Pin<Box<dyn Read + Send>> {
    #[instrument(skip(self))]
    async fn read_byte(&mut self) -> io::Result<u8> {
        let mut buffer = [0; 1];
        self.as_mut().read_exact(&mut buffer).await?;
        trace!("Read byte: {:02x}", buffer[0]);
        Ok(buffer[0])
    }

    #[instrument(skip(self, buf))]
    async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        let result = self.as_mut().read_exact(buf).await;
        if let Err(ref e) = result {
            error!("Error reading exact bytes: {}", e);
        } else {
            trace!("Read {} bytes", buf.len());
        }
        result
    }
}

/// Writes a protocol buffer message to a writer.
///
/// This function encodes the message, writes its length as a varint, and then
/// writes the encoded message data.
///
/// # Arguments
///
/// * `msg` - The protocol buffer message to write.
/// * `w` - The writer to write the message to.
///
/// # Returns
///
/// A `Result` indicating success or an I/O error.
///
/// # Examples
///
/// ```
/// use async_std::io::Cursor;
/// use prost::Message;
/// use your_crate::io::write_message;
///
/// #[derive(Message)]
/// struct MyMessage {
///     #[prost(int32, tag="1")]
///     id: i32,
/// }
///
/// # async fn example() -> std::io::Result<()> {
/// let msg = MyMessage { id: 42 };
/// let mut writer = Cursor::new(Vec::new());
/// write_message(&msg, &mut writer).await?;
/// # Ok(())
/// # }
/// ```
#[instrument(skip(msg, w))]
pub async fn write_message<W: Write + Unpin, M: Message>(msg: &M, mut w: W) -> io::Result<()> {
    let data = msg.encode_to_vec();
    debug!("Encoded message length: {}", data.len());
    write_varint(&mut w, data.len() as u64).await?;
    debug!("Wrote message length");
    w.write_all(&data).await?;
    debug!("Wrote message data");
    Ok(())
}

/// Reads a protocol buffer message from a reader.
///
/// This function reads the message length as a varint, reads the message data,
/// and then decodes the message.
///
/// # Arguments
///
/// * `buf` - A buffer to use for reading the message data.
/// * `r` - The reader to read the message from.
/// * `msg` - The protocol buffer message to decode into.
///
/// # Returns
///
/// A `Result` indicating success or an I/O error.
///
/// # Examples
///
/// ```
/// use async_std::io::Cursor;
/// use prost::Message;
/// use your_crate::io::read_message;
///
/// #[derive(Message, Default)]
/// struct MyMessage {
///     #[prost(int32, tag="1")]
///     id: i32,
/// }
///
/// # async fn example() -> std::io::Result<()> {
/// let mut reader = Cursor::new(vec![/* encoded message data */]);
/// let mut buf = Vec::new();
/// let mut msg = MyMessage::default();
/// read_message(&mut buf, &mut reader, &mut msg).await?;
/// # Ok(())
/// # }
/// ```
#[instrument(skip(buf, r, msg))]
pub async fn read_message<R: Read + Unpin>(
    buf: &mut Vec<u8>,
    r: &mut R,
    msg: &mut impl prost::Message,
) -> io::Result<()> {
    let size = read_uvarint(r).await?;
    debug!("Reading message of size: {}", size);

    buf.resize(size as usize, 0);

    r.read_exact(buf).await?;
    trace!("Read {} bytes into buffer", size);

    msg.clear();
    msg.merge(&buf[..]).map_err(|e| {
        error!("Error merging message: {}", e);
        io::Error::new(io::ErrorKind::InvalidData, e)
    })?;

    debug!("Message read and parsed successfully");
    Ok(())
}

/// Reads an unsigned varint from a reader.
///
/// # Arguments
///
/// * `r` - The reader to read the varint from.
///
/// # Returns
///
/// A `Result` containing the read varint or an I/O error.
#[instrument(skip(r))]
async fn read_uvarint<R: Read + Unpin>(r: &mut R) -> io::Result<u64> {
    let mut x = 0u64;
    let mut s = 0u32;
    for i in 0..10 {
        let mut buf = [0u8];
        r.read_exact(&mut buf).await?;
        let b = buf[0];
        if b < 0x80 {
            if i == 9 && b > 1 {
                warn!("Varint overflow detected");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "varint overflows a 64-bit integer",
                ));
            }
            let result = x | (u64::from(b) << s);
            trace!("Read uvarint: {}", result);
            return Ok(result);
        }
        x |= u64::from(b & 0x7f) << s;
        s += 7;
    }
    warn!("Varint too long");
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "varint too long",
    ))
}

/// Writes an unsigned varint to a writer.
///
/// # Arguments
///
/// * `w` - The writer to write the varint to.
/// * `value` - The value to write as a varint.
///
/// # Returns
///
/// A `Result` indicating success or an I/O error.
#[instrument(skip(w))]
async fn write_varint<W: Write + Unpin>(w: &mut W, mut value: u64) -> io::Result<()> {
    trace!("Writing varint: {}", value);
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        w.write_all(&[byte]).await?;
        if value == 0 {
            break;
        }
    }
    trace!("Varint written successfully");
    Ok(())
}
