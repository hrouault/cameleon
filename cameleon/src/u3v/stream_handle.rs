/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This module contains low level streaming implementation for `U3V` device.

use crate::{
    camera::PayloadStream,
    payload::{ImageInfo, Payload, PayloadType},
    ControlError, ControlResult, DeviceControl, StreamError, StreamResult,
};
use cameleon_device::u3v::{
    self,
    protocol::stream::{self as u3v_stream, Leader, Trailer},
};
use futures_lite::{stream, Stream};
use nusb::transfer::{Queue, RequestBuffer};
use std::{sync::mpsc::Receiver, time::Duration};
use tracing::{error, info};

use super::register_map::Abrm;

/// This type is used to receive stream packets from the device.
pub struct StreamHandle {
    /// Inner channel to receive payload data.
    pub stream_channel: u3v::ReceiveChannel,
    /// Parameters for streaming.
    params: StreamParams,

    payload_rx: Option<Receiver<Vec<u8>>>,

    leader_buf: Option<Vec<u8>>,
    trailer_buf: Option<Vec<u8>>,
    final1_buf: Option<Vec<u8>>,
    final2_buf: Option<Vec<u8>>,
    payload_bufs: Vec<Vec<u8>>,
    pic_buf: Option<Vec<u8>>,
}

impl StreamHandle {
    pub(super) fn new(device: &u3v::Device) -> ControlResult<Option<Self>> {
        info!("get stream channel");
        let channel = device.stream_channel()?;

        Ok(channel.map(|channel| Self {
            stream_channel: channel,
            params: StreamParams::default(),

            payload_rx: None,

            leader_buf: None,
            trailer_buf: None,
            final1_buf: None,
            final2_buf: None,
            payload_bufs: Vec::<Vec<u8>>::default(),
            pic_buf: None,
        }))
    }

    /// Return params.
    #[must_use]
    pub fn params(&self) -> &StreamParams {
        &self.params
    }

    ///  Return mutable params.
    pub fn params_mut(&mut self) -> &mut StreamParams {
        &mut self.params
    }

    fn submit_leader(&mut self, queue: &mut Queue<RequestBuffer>) -> StreamResult<()> {
        let req_buf = if let Some(buf) = self.leader_buf.take() {
            RequestBuffer::reuse(buf, self.params.leader_size)
        } else {
            RequestBuffer::new(self.params.leader_size)
        };
        queue.submit(req_buf);

        Ok(())
    }

    fn submit_payload(&mut self, queue: &mut Queue<RequestBuffer>) -> StreamResult<()> {
        let payload_size = self.params.payload_size;
        for _ in 0..self.params.payload_count {
            let req_buf = if let Some(buf) = self.payload_bufs.pop() {
                RequestBuffer::reuse(buf, payload_size)
            } else {
                RequestBuffer::new(payload_size)
            };
            queue.submit(req_buf);
        }

        if self.params.payload_final1_size != 0 {
            let req_buf = if let Some(buf) = self.final1_buf.take() {
                RequestBuffer::reuse(buf, self.params.payload_final1_size)
            } else {
                RequestBuffer::new(self.params.payload_final1_size)
            };
            queue.submit(req_buf);
        }
        if self.params.payload_final2_size != 0 {
            let req_buf = if let Some(buf) = self.final2_buf.take() {
                RequestBuffer::reuse(buf, self.params.payload_final2_size)
            } else {
                RequestBuffer::new(self.params.payload_final2_size)
            };
            queue.submit(req_buf);
        }

        Ok(())
    }

    fn submit_trailer(&mut self, queue: &mut Queue<RequestBuffer>) -> StreamResult<()> {
        let req_buf = if let Some(buf) = self.trailer_buf.take() {
            RequestBuffer::reuse(buf, self.params.trailer_size)
        } else {
            RequestBuffer::new(self.params.trailer_size)
        };
        queue.submit(req_buf);

        Ok(())
    }

    async fn read_leader(&mut self, queue: &mut Queue<RequestBuffer>) -> StreamResult<()> {
        let leader_buf = queue.next_complete().await.into_result()?;
        self.leader_buf = Some(leader_buf);
        Ok(())
    }

    fn parse_leader(&self) -> StreamResult<Leader> {
        match &self.leader_buf {
            Some(buf) => Ok(u3v_stream::Leader::parse(&buf[..])?),
            None => Err(StreamError::NoBuffer),
        }
    }

    async fn read_payload(&mut self, queue: &mut Queue<RequestBuffer>) -> StreamResult<()> {
        let maximum_payload_size = self.params.maximum_payload_size();
        let mut pic_buf = match self.pic_buf.take() {
            Some(mut buf) => {
                if buf.len() != maximum_payload_size {
                    buf.resize(maximum_payload_size, 0);
                }
                buf
            }
            None => {
                if let Some(pay_rx) = &self.payload_rx {
                    if let Ok(buf) = pay_rx.try_recv() {
                        buf
                    } else {
                        vec![0; maximum_payload_size]
                    }
                } else {
                    vec![0; maximum_payload_size]
                }
            }
        };

        let mut cursor = 0;
        for _i in 0..self.params.payload_count {
            let payload_size = self.params.payload_size;
            let completion = queue.next_complete().await.into_result()?;
            pic_buf[cursor..cursor + payload_size].clone_from_slice(&completion);
            cursor += payload_size;
            self.payload_bufs.push(completion);
        }
        let final1 = self.params.payload_final1_size;
        if final1 != 0 {
            let completion = queue.next_complete().await.into_result()?;
            pic_buf[cursor..cursor + final1].clone_from_slice(&completion);
            cursor += final1;
            self.final1_buf = Some(completion);
        }
        let final2 = self.params.payload_final2_size;
        if final2 != 0 {
            let completion = queue.next_complete().await.into_result()?;
            pic_buf[cursor..cursor + final1].clone_from_slice(&completion);
            self.final2_buf = Some(completion);
        }
        self.pic_buf = Some(pic_buf);
        Ok(())
    }

    async fn read_trailer(&mut self, queue: &mut Queue<RequestBuffer>) -> StreamResult<()> {
        let trailer_buf = queue.next_complete().await.into_result()?;
        self.trailer_buf = Some(trailer_buf);
        Ok(())
    }

    fn parse_trailer(&self) -> StreamResult<Trailer> {
        match &self.trailer_buf {
            Some(buf) => Ok(u3v_stream::Trailer::parse(&buf[..])?),
            None => Err(StreamError::NoBuffer),
        }
    }

    async fn next_payload(&mut self) -> Result<Payload, StreamError> {
        let mut queue = {
            let channel = &self.stream_channel;
            let iface = channel.iface.as_ref().unwrap();

            iface.bulk_in_queue(channel.iface_info.bulk_in_ep)
        };

        // read leader
        self.submit_leader(&mut queue)?;
        self.submit_payload(&mut queue)?;
        self.submit_trailer(&mut queue)?;

        // We've submitted the bulk transfers, now wait for them and parse the results
        // parse the leader
        self.read_leader(&mut queue).await?;
        self.read_payload(&mut queue).await?;
        self.read_trailer(&mut queue).await?;
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

impl PayloadStream for StreamHandle {
    fn open(&mut self) -> StreamResult<()> {
        self.stream_channel.open().map_err(|e| {
            error!(?e);
            e.into()
        })
    }

    /// Get an async stream that return a stream of payload results
    fn start_streaming(
        &mut self,
        ctrl: &mut dyn DeviceControl,
        payload_rx: Receiver<Vec<u8>>,
    ) -> StreamResult<impl Stream<Item = StreamResult<Payload>>> {
        self.params = StreamParams::from_control(ctrl).map_err(|e| {
            StreamError::Io(anyhow::Error::msg(format!(
                "failed to setup streaming parameters: {}",
                e
            )))
        })?;

        self.payload_rx = Some(payload_rx);

        Ok(stream::unfold(self, |stream| async move {
            let payload = stream.next_payload().await;
            Some((payload, stream))
        }))
    }

    fn reuse_payload(&mut self, payload: Vec<u8>) -> Result<(), StreamError> {
        self.pic_buf = Some(payload);
        Ok(())
    }
}

struct PayloadBuilder<'a> {
    leader: u3v_stream::Leader<'a>,
    payload_buf: Vec<u8>,
    read_payload_size: usize,
    trailer: u3v_stream::Trailer<'a>,
}

impl<'a> PayloadBuilder<'a> {
    fn build(self) -> StreamResult<Payload> {
        let payload_status = self.trailer.payload_status();
        if payload_status != u3v_stream::PayloadStatus::Success {
            return Err(StreamError::InvalidPayload(
                format!("trailer status indicates error: {:?}", payload_status).into(),
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
            .map_err(|e| StreamError::InvalidPayload(format!("{}", e).into()))
    }

    fn specific_trailer_as<T: u3v_stream::SpecificTrailer>(&self) -> StreamResult<T> {
        self.trailer
            .specific_trailer_as()
            .map_err(|e| StreamError::InvalidPayload(format!("{}", e).into()))
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
