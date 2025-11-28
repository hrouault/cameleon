/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This module contains low level streaming implementation for `U3V` device.

use crate::{
    camera::StreamInterface,
    payload::{ImageInfo, Payload, PayloadType},
    ControlError, ControlResult, DeviceControl, StreamError, StreamResult,
};
use cameleon_device::u3v::{
    self,
    protocol::stream::{self as u3v_stream, Leader, Trailer},
};
use futures_lite::future::block_on;
use futures_lite::Stream;
use pin_project_lite::pin_project;
use std::{
    future::Future,
    pin::Pin,
    sync::mpsc::Receiver,
    task::{ready, Context, Poll},
    time::Duration,
};
use tracing::{debug, error, info};

use nusb::{
    transfer::{Buffer, Bulk, In},
    Endpoint,
};

use super::register_map::Abrm;

/// This type is used to receive stream packets from the device.
pub struct StreamHandle {
    /// Inner channel to receive payload data.
    pub stream_channel: u3v::ReceiveChannel,
}

impl StreamHandle {
    pub(super) fn new(device: &u3v::Device) -> ControlResult<Option<Self>> {
        info!("get stream channel");
        let channel = device.stream_channel()?;

        Ok(channel.map(|channel| Self {
            stream_channel: channel,
        }))
    }
}

impl StreamInterface for StreamHandle {
    fn open(&mut self) -> StreamResult<()> {
        block_on(self.stream_channel.open()).map_err(|e| {
            error!(?e);
            e.into()
        })
    }

    /// Get an async stream that return a stream of payload results
    fn start_streaming(
        &self,
        ctrl: &mut dyn DeviceControl,
        payload_rx: Receiver<Vec<u8>>,
    ) -> StreamResult<PayloadStream> {
        let params = StreamParams::from_control(ctrl).map_err(StreamError::StreamParams)?;

        let iface = self
            .stream_channel
            .iface
            .clone()
            .ok_or_else(|| StreamError::NoInterface)?;

        // We expect this endpoint to exist; you can replace `unwrap` with better error mapping.
        let endpoint = iface
            .endpoint::<Bulk, In>(self.stream_channel.iface_info.bulk_in_ep)
            .expect("failed to open bulk IN endpoint for streaming");

        let max_packet_size = endpoint.max_packet_size() as usize;

        Ok(PayloadStream {
            state: PayloadStreamState::Value {
                value: PayloadStreamInner {
                    params,
                    payload_rx,
                    endpoint,
                    max_packet_size,
                    leader_buf: None,
                    trailer_buf: None,
                    final1_buf: None,
                    final2_buf: None,
                    payload_bufs: Vec::new(),
                    pic_buf: None,
                },
            },
        })
    }

    /// Get an async stream that return a stream of payload results
    fn start_generator(
        &self,
        ctrl: &mut dyn DeviceControl,
        payload_rx: Receiver<Vec<u8>>,
    ) -> StreamResult<PayloadGenerator> {
        let params = StreamParams::from_control(ctrl).map_err(StreamError::StreamParams)?;

        let iface = self
            .stream_channel
            .iface
            .clone()
            .ok_or_else(|| StreamError::NoInterface)?;

        let endpoint = iface
            .endpoint::<Bulk, In>(self.stream_channel.iface_info.bulk_in_ep)
            .expect("failed to open bulk IN endpoint for streaming");

        let max_packet_size = endpoint.max_packet_size() as usize;

        Ok(PayloadGenerator {
            inner: PayloadStreamInner {
                params,
                payload_rx,
                endpoint,
                max_packet_size,
                leader_buf: None,
                trailer_buf: None,
                final1_buf: None,
                final2_buf: None,
                payload_bufs: Vec::new(),
                pic_buf: None,
            },
        })
    }
}

impl From<StreamHandle> for Box<dyn StreamInterface> {
    fn from(handle: StreamHandle) -> Self {
        Box::new(handle)
    }
}

pub(crate) struct PayloadStreamInner {
    params: StreamParams,
    payload_rx: Receiver<Vec<u8>>,
    endpoint: Endpoint<Bulk, In>,
    max_packet_size: usize,
    leader_buf: Option<Buffer>,
    trailer_buf: Option<Buffer>,
    final1_buf: Option<Buffer>,
    final2_buf: Option<Buffer>,
    payload_bufs: Vec<Buffer>,
    pic_buf: Option<Vec<u8>>,
}

impl PayloadStreamInner {
    #[inline]
    fn align_up(&self, len: usize) -> usize {
        if len == 0 {
            0
        } else if len % self.max_packet_size == 0 {
            len
        } else {
            ((len / self.max_packet_size) + 1) * self.max_packet_size
        }
    }

    fn submit_leader(&mut self) -> StreamResult<()> {
        let logical = self.params.leader_size;
        let req_len = self.align_up(logical);

        let mut buf = if let Some(buf) = self.leader_buf.take() {
            if buf.capacity() < req_len {
                Buffer::new(req_len)
            } else {
                buf
            }
        } else {
            Buffer::new(req_len)
        };

        buf.clear();
        buf.set_requested_len(req_len);
        self.endpoint.submit(buf);

        debug!(
            "Leader size: logical = {}, usb_requested = {}",
            logical, req_len
        );

        Ok(())
    }

    fn submit_payload(&mut self) -> StreamResult<()> {
        let payload_size = self.params.payload_size;
        let payload_req = self.align_up(payload_size);

        // Full payload chunks
        for _ in 0..self.params.payload_count {
            let mut buf = if let Some(buf) = self.payload_bufs.pop() {
                if buf.capacity() < payload_req {
                    Buffer::new(payload_req)
                } else {
                    buf
                }
            } else {
                Buffer::new(payload_req)
            };
            buf.clear();
            buf.set_requested_len(payload_req);
            self.endpoint.submit(buf);
            debug!(
                "Payload size: logical = {}, usb_requested = {}",
                payload_size, payload_req
            );
        }

        // Final1
        let final1 = self.params.payload_final1_size;
        if final1 != 0 {
            let final1_req = self.align_up(final1);
            let mut buf = if let Some(buf) = self.final1_buf.take() {
                if buf.capacity() < final1_req {
                    Buffer::new(final1_req)
                } else {
                    buf
                }
            } else {
                Buffer::new(final1_req)
            };
            buf.clear();
            buf.set_requested_len(final1_req);
            self.endpoint.submit(buf);
            debug!(
                "final1 size: logical = {}, usb_requested = {}",
                final1, final1_req
            );
        } else {
            debug!("final1 size: 0");
        }

        // Final2
        let final2 = self.params.payload_final2_size;
        if final2 != 0 {
            let final2_req = self.align_up(final2);
            let mut buf = if let Some(buf) = self.final2_buf.take() {
                if buf.capacity() < final2_req {
                    Buffer::new(final2_req)
                } else {
                    buf
                }
            } else {
                Buffer::new(final2_req)
            };
            buf.clear();
            buf.set_requested_len(final2_req);
            self.endpoint.submit(buf);
            debug!(
                "final2 size: logical = {}, usb_requested = {}",
                final2, final2_req
            );
        } else {
            debug!("final2 size: 0");
        }

        Ok(())
    }

    fn submit_trailer(&mut self) -> StreamResult<()> {
        let logical = self.params.trailer_size;
        let req_len = self.align_up(logical);

        let mut buf = if let Some(buf) = self.trailer_buf.take() {
            if buf.capacity() < req_len {
                Buffer::new(req_len)
            } else {
                buf
            }
        } else {
            Buffer::new(req_len)
        };
        buf.clear();
        buf.set_requested_len(req_len);
        self.endpoint.submit(buf);
        debug!(
            "trailer size: logical = {}, usb_requested = {}",
            logical, req_len
        );

        Ok(())
    }

    async fn read_leader(&mut self) -> StreamResult<()> {
        let leader_buf = self.endpoint.next_complete().await.into_result()?;
        self.leader_buf = Some(leader_buf);
        Ok(())
    }

    fn parse_leader(&self) -> StreamResult<Leader<'_>> {
        match &self.leader_buf {
            Some(buf) => Ok(u3v_stream::Leader::parse(&buf[..])?),
            None => Err(StreamError::NoBuffer),
        }
    }

    async fn read_payload(&mut self) -> StreamResult<()> {
        let maximum_payload_size = self.params.maximum_payload_size();
        let mut pic_buf = match self.pic_buf.take() {
            Some(mut buf) => {
                if buf.len() != maximum_payload_size {
                    buf.resize(maximum_payload_size, 0);
                }
                buf
            }
            None => {
                if let Ok(buf) = self.payload_rx.try_recv() {
                    buf
                } else {
                    vec![0; maximum_payload_size]
                }
            }
        };

        let mut cursor = 0;

        // Full chunks
        for _ in 0..self.params.payload_count {
            let logical = self.params.payload_size;
            debug!(
                "Waiting for completion: {}, {} pending transfers",
                cursor,
                self.endpoint.pending()
            );
            let completion = self.endpoint.next_complete().await.into_result()?;
            let got = completion.len();
            let to_copy = logical.min(got);

            if cursor + to_copy > pic_buf.len() {
                return Err(StreamError::InvalidPayload(
                    "payload buffer overflow while copying full chunks".into(),
                ));
            }

            pic_buf[cursor..cursor + to_copy].copy_from_slice(&completion[..to_copy]);
            cursor += to_copy;
            self.payload_bufs.push(completion);
        }

        // Final1
        let final1 = self.params.payload_final1_size;
        if final1 != 0 {
            debug!(
                "Waiting for completion: {}, {} pending transfers",
                cursor,
                self.endpoint.pending()
            );
            let completion = self.endpoint.next_complete().await.into_result()?;
            let got = completion.len();
            let to_copy = final1.min(got);

            if cursor + to_copy > pic_buf.len() {
                return Err(StreamError::InvalidPayload(
                    "payload buffer overflow while copying final1".into(),
                ));
            }

            pic_buf[cursor..cursor + to_copy].copy_from_slice(&completion[..to_copy]);
            cursor += to_copy;
            self.final1_buf = Some(completion);
        }

        // Final2
        let final2 = self.params.payload_final2_size;
        if final2 != 0 {
            debug!(
                "Waiting for completion: {}, {} pending transfers",
                cursor,
                self.endpoint.pending()
            );
            let completion = self.endpoint.next_complete().await.into_result()?;
            let got = completion.len();
            let to_copy = final2.min(got);

            if cursor + to_copy > pic_buf.len() {
                return Err(StreamError::InvalidPayload(
                    "payload buffer overflow while copying final2".into(),
                ));
            }

            pic_buf[cursor..cursor + to_copy].copy_from_slice(&completion[..to_copy]);
            cursor += to_copy;
            self.final2_buf = Some(completion);
        }

        self.pic_buf = Some(pic_buf);
        Ok(())
    }

    async fn read_trailer(&mut self) -> StreamResult<()> {
        debug!(
            "Waiting for completion trailer, {} pending transfers",
            self.endpoint.pending()
        );
        let trailer_buf = self.endpoint.next_complete().await.into_result()?;
        self.trailer_buf = Some(trailer_buf);
        Ok(())
    }

    fn parse_trailer(&self) -> StreamResult<Trailer<'_>> {
        match &self.trailer_buf {
            Some(buf) => Ok(u3v_stream::Trailer::parse(&buf[..])?),
            None => Err(StreamError::NoBuffer),
        }
    }

    async fn next_payload(&mut self) -> Result<Payload, StreamError> {
        // read leader
        self.submit_leader()?;
        self.submit_payload()?;
        self.submit_trailer()?;
        debug!("Submitted all the packets");

        // We've submitted the bulk transfers, now wait for them and parse the results
        // parse the leader
        self.read_leader().await?;
        debug!("Received the leader");
        self.read_payload().await?;
        debug!("Received the main payload");
        self.read_trailer().await?;
        debug!("Received all the packets");
        let pic_buf = self.pic_buf.take().ok_or_else(|| StreamError::NoBuffer)?;

        let leader = self.parse_leader()?;
        let trailer = self.parse_trailer()?;

        let read_payload_size = pic_buf.len();
        PayloadBuilder {
            leader,
            payload_buf: pic_buf,
            read_payload_size,
            trailer,
        }
        .build()
    }
}

pin_project! {
    /// PayloadStream structure that implement the Stream trait
    pub struct PayloadStream {
        #[pin]
        state: PayloadStreamState,
    }
}

type BoxedFut =
    Pin<Box<dyn std::future::Future<Output = (StreamResult<Payload>, PayloadStreamInner)> + Send>>;
pin_project! {
    #[project = PayloadStreamStateProj]
    #[project_replace = PayloadStreamStateProjReplace]
    enum PayloadStreamState {
        Value {
            value: PayloadStreamInner,
        },
        Future {
            #[pin]
            future: BoxedFut,
        },
        Empty,
    }
}

impl PayloadStreamState {
    pub(crate) fn project_future(self: Pin<&mut Self>) -> Option<Pin<&mut BoxedFut>> {
        match self.project() {
            PayloadStreamStateProj::Future { future } => Some(future),
            _ => None,
        }
    }

    pub(crate) fn take_value(self: Pin<&mut Self>) -> Option<PayloadStreamInner> {
        match &*self {
            Self::Value { .. } => match self.project_replace(Self::Empty) {
                PayloadStreamStateProjReplace::Value { value } => Some(value),
                _ => unreachable!(),
            },
            _ => None,
        }
    }
}

impl Stream for PayloadStream {
    type Item = StreamResult<Payload>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if let Some(mut state) = this.state.as_mut().take_value() {
            this.state.set(PayloadStreamState::Future {
                future: Box::pin(async move { (state.next_payload().await, state) }),
            });
        }

        let step = match this.state.as_mut().project_future() {
            Some(fut) => ready!(fut.poll(cx)),
            None => panic!("Unfold must not be polled after it returned `Poll::Ready(None)`"),
        };

        this.state.set(PayloadStreamState::Value { value: step.1 });
        Poll::Ready(Some(step.0))
    }
}

/// A helper type that produces payloads one-by-one on demand.
///
/// Unlike [`PayloadStream`], which implements [`futures_core::Stream`]
/// and can be used in `while let Some(...) = stream.next().await` loops,
/// `PayloadGenerator` exposes a simple async method to fetch the next
/// payload when you want it.
///
/// This is useful if you don't want to depend on the `Stream` trait
/// or prefer a more manual, pull-based API.
pub struct PayloadGenerator {
    inner: PayloadStreamInner,
}

impl PayloadGenerator {
    /// Asynchronously retrieves the next [`Payload`] from the stream.
    ///
    /// This method will:
    /// - submit the required USB transfers,
    /// - wait for them to complete,
    /// - assemble and parse the U3V leader, payload and trailer,
    /// - and return a fully constructed [`Payload`].
    ///
    /// Returns an error if the underlying USB transfer or U3V parsing fails.
    pub async fn next_payload(&mut self) -> StreamResult<Payload> {
        self.inner.next_payload().await
    }
}

struct PayloadBuilder<'a> {
    leader: u3v_stream::Leader<'a>,
    payload_buf: Vec<u8>,
    read_payload_size: usize,
    trailer: u3v_stream::Trailer<'a>,
}

impl PayloadBuilder<'_> {
    fn build(self) -> StreamResult<Payload> {
        let payload_status = self.trailer.payload_status();
        if payload_status != u3v_stream::PayloadStatus::Success {
            return Err(StreamError::InvalidPayload(
                format!("trailer status indicates error: {payload_status:?}").into(),
            ));
        }

        if self.trailer.valid_payload_size() > self.read_payload_size as u64 {
            let err_msg = format!("the actual read payload size is smaller than the size specified in the trailer: expected {}, but got {}",
                                  self.trailer.valid_payload_size(),
                                  self.read_payload_size);
            return Err(StreamError::InvalidPayload(err_msg.into()));
        }

        match self.leader.payload_type() {
            u3v_stream::PayloadType::Image => self.build_image_payload(),
            u3v_stream::PayloadType::ImageExtendedChunk => self.build_image_extended_payload(),
            u3v_stream::PayloadType::Chunk => self.build_chunk_payload(),
        }
    }

    fn build_image_payload(self) -> StreamResult<Payload> {
        let leader: u3v_stream::ImageLeader = self.specific_leader_as()?;
        let trailer: u3v_stream::ImageTrailer = self.specific_trailer_as()?;

        let id = self.leader.block_id();
        let valid_payload_size = self.trailer.valid_payload_size() as usize;

        let image_info = Some(ImageInfo {
            width: leader.width() as usize,
            height: trailer.actual_height() as usize,
            x_offset: leader.x_offset() as usize,
            y_offset: leader.y_offset() as usize,
            pixel_format: leader.pixel_format(),
            image_size: valid_payload_size,
        });

        Ok(Payload {
            id,
            payload_type: PayloadType::Image,
            image_info,
            payload: self.payload_buf,
            valid_payload_size,
            timestamp: leader.timestamp(),
        })
    }

    fn build_image_extended_payload(self) -> StreamResult<Payload> {
        const CHUNK_ID_LEN: usize = 4;
        const CHUNK_SIZE_LEN: usize = 4;

        let leader: u3v_stream::ImageExtendedChunkLeader = self.specific_leader_as()?;
        let trailer: u3v_stream::ImageExtendedChunkTrailer = self.specific_trailer_as()?;

        let id = self.leader.block_id();
        let valid_payload_size = self.trailer.valid_payload_size() as usize;

        // Extract image size from the first chunk of the paload data.
        // Chunk data is designed to be decoded from the last byte to the first byte.
        // Use chunk parser of `cameleon_genapi` once it gets implemented.
        let mut current_offset = valid_payload_size;
        let image_size = loop {
            current_offset = current_offset.checked_sub(CHUNK_SIZE_LEN).ok_or_else(|| {
                StreamError::InvalidPayload("failed to parse chunk data: size field missing".into())
            })?;
            let data_size = u32::from_be_bytes(
                self.payload_buf[current_offset..current_offset + CHUNK_SIZE_LEN]
                    .try_into()
                    .unwrap(),
            ) as usize;
            current_offset = current_offset.checked_sub(data_size + CHUNK_ID_LEN).ok_or_else(|| {
                StreamError::InvalidPayload(
                    "failed to parse chunk data: chunk data size is smaller than specified size".into()
                )
            })?;

            if current_offset == 0 {
                break data_size;
            }
        };

        let image_info = Some(ImageInfo {
            width: leader.width() as usize,
            height: trailer.actual_height() as usize,
            x_offset: leader.x_offset() as usize,
            y_offset: leader.y_offset() as usize,
            pixel_format: leader.pixel_format(),
            image_size,
        });

        Ok(Payload {
            id,
            payload_type: PayloadType::ImageExtendedChunk,
            image_info,
            payload: self.payload_buf,
            valid_payload_size,
            timestamp: leader.timestamp(),
        })
    }

    fn build_chunk_payload(self) -> StreamResult<Payload> {
        let leader: u3v_stream::ChunkLeader = self.specific_leader_as()?;
        let _: u3v_stream::ChunkTrailer = self.specific_trailer_as()?;

        let id = self.leader.block_id();
        let valid_payload_size = self.trailer.valid_payload_size() as usize;

        Ok(Payload {
            id,
            payload_type: PayloadType::Chunk,
            image_info: None,
            payload: self.payload_buf,
            valid_payload_size,
            timestamp: leader.timestamp(),
        })
    }

    fn specific_leader_as<T: u3v_stream::SpecificLeader>(&self) -> StreamResult<T> {
        self.leader
            .specific_leader_as()
            .map_err(|e| StreamError::InvalidPayload(format!("{e}").into()))
    }

    fn specific_trailer_as<T: u3v_stream::SpecificTrailer>(&self) -> StreamResult<T> {
        self.trailer
            .specific_trailer_as()
            .map_err(|e| StreamError::InvalidPayload(format!("{e}").into()))
    }
}

/// Parameters to receive stream packets.
///
/// Both [`StreamHandle`] doesn't check the integrity of the parameters. That's up to user.
#[derive(Debug, Clone, Default)]
pub struct StreamParams {
    /// Maximum leader size.
    pub leader_size: usize,

    /// Maximum trailer size.
    pub trailer_size: usize,

    /// Payload transfer size.
    pub payload_size: usize,

    /// Payload transfer count.
    pub payload_count: usize,

    /// Payload transfer final1 size.
    pub payload_final1_size: usize,

    /// Payload transfer final2 size.
    pub payload_final2_size: usize,

    /// Timeout duration of each transaction between device.
    pub timeout: Duration,
}

impl StreamParams {
    /// Return upper bound of payload size calculated by current `StreamParams` values.
    ///
    /// NOTE: Payload size may dynamically change according to settings of camera.
    pub fn maximum_payload_size(&self) -> usize {
        self.payload_size * self.payload_count + self.payload_final1_size + self.payload_final2_size
    }
}

impl StreamParams {
    /// Construct `StreamParams`.
    #[must_use]
    pub fn new(
        leader_size: usize,
        trailer_size: usize,
        payload_size: usize,
        payload_count: usize,
        payload_final1_size: usize,
        payload_final2_size: usize,
        timeout: Duration,
    ) -> Self {
        Self {
            leader_size,
            trailer_size,
            payload_size,
            payload_count,
            payload_final1_size,
            payload_final2_size,
            timeout,
        }
    }

    /// Build `StreamParams` from [`DeviceControl`].
    pub fn from_control<Ctrl: DeviceControl + ?Sized>(ctrl: &mut Ctrl) -> ControlResult<Self> {
        let abrm = Abrm::new(ctrl)?;
        let sirm = abrm.sbrm(ctrl)?.sirm(ctrl)?.ok_or_else(|| {
            let msg = "the U3V device doesn't have `SIRM`";
            error!(msg);
            ControlError::InvalidDevice(msg.into())
        })?;
        let leader_size = sirm.maximum_leader_size(ctrl)? as usize;
        let trailer_size = sirm.maximum_trailer_size(ctrl)? as usize;

        let payload_size = sirm.payload_transfer_size(ctrl)? as usize;
        let payload_count = sirm.payload_transfer_count(ctrl)? as usize;
        let payload_final1_size = sirm.payload_final_transfer1_size(ctrl)? as usize;
        let payload_final2_size = sirm.payload_final_transfer2_size(ctrl)? as usize;
        let timeout = abrm.maximum_device_response_time(ctrl)?;

        Ok(Self::new(
            leader_size,
            trailer_size,
            payload_size,
            payload_count,
            payload_final1_size,
            payload_final2_size,
            timeout,
        ))
    }
}
