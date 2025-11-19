/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This module contains types related to `Payload` sent from the device.
//!
//! `Payload` is an abstracted container that is mainly used to transfer an image, but also meta data of the image.
//! See [`Payload`] and [`ImageInfo`] for more details.

pub use cameleon_device::PixelFormat;

use std::time;

/// Represents Payload type of the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadType {
    /// Payload contains just an image data only.
    Image,
    /// Payload contains multiple data chunks, and its first chunk is an image.
    ImageExtendedChunk,
    /// Payload contains multiple data chunks, no gurantee about its first chunk.
    Chunk,
}

/// Image meta information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// Width of the image.
    pub width: usize,
    /// Height of the image.
    pub height: usize,
    /// X offset in pixels from the whole image origin. Some devices have capability of
    /// sending multiple extracted image regions, this fields used for the purpose.
    pub x_offset: usize,
    /// Y offset in pixels from the whole image origin. Some devices have capability of
    /// sending multiple extracted image regions, this fields used for the purpose.
    pub y_offset: usize,
    /// [`PixelFormat`] of the image.
    pub pixel_format: PixelFormat,
    /// Size of image in bytes.
    pub image_size: usize,
}

/// A payload sent from the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    pub(crate) id: u64,
    pub(crate) payload_type: PayloadType,
    pub(crate) image_info: Option<ImageInfo>,
    pub(crate) payload: Vec<u8>,
    pub(crate) valid_payload_size: usize,
    pub(crate) timestamp: time::Duration,
}

impl Payload {
    /// Returns [`PayloadType`] of the payload.
    pub fn payload_type(&self) -> PayloadType {
        self.payload_type
    }

    /// Returns [`ImageInfo`] if `payload_type` is [`PayloadType::Image`] or
    /// [`PayloadType::ImageExtendedChunk`].
    pub fn image_info(&self) -> Option<&ImageInfo> {
        self.image_info.as_ref()
    }

    /// Returns the image bytes in the payload if `payload_type` is [`PayloadType::Image`]  or
    /// [`PayloadType::ImageExtendedChunk`].
    pub fn image(&self) -> Option<&[u8]> {
        let image_info = self.image_info()?;
        Some(&self.payload[..image_info.image_size])
    }

    /// Returns the whole payload. Use [`Self::image`] instead if you interested only
    /// in image region of the payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.valid_payload_size]
    }

    /// Consume this `Payload` and return the underlying buffer so it can be
    /// reused by the streaming code.
    pub fn into_reuse_buffer(self) -> Vec<u8> {
        self.payload
    }

    /// Convenience helper: send the internal buffer to a reuse channel.
    ///
    /// If the receiver has been dropped, the error is ignored.
    pub fn return_buffer(self, reuse_tx: &std::sync::mpsc::Sender<Vec<u8>>) {
        let buf = self.into_reuse_buffer();
        let _ = reuse_tx.send(buf);
    }

    /// Returns unique id of `payload`, which sequentially incremented every time the device send a
    /// `payload`.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Timestamp of the device when the payload is generated.
    pub fn timestamp(&self) -> time::Duration {
        self.timestamp
    }

    /// Returns the payload as `Vec<u8>`.
    pub fn into_vec(mut self) -> Vec<u8> {
        self.payload.resize(self.valid_payload_size, 0);
        self.payload
    }
}
