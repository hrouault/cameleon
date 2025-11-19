/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::u3v::{U3vError, U3vResult};
use nusb::{
    io::{EndpointRead, EndpointWrite},
    transfer::{Bulk, In, Out},
    Device, Interface,
};

pub struct ControlChannel {
    pub(super) device: Device,
    pub iface_info: ControlIfaceInfo,
    pub iface: Option<Interface>,
    tx: Option<EndpointWrite<Bulk>>,
    rx: Option<EndpointRead<Bulk>>,
}

impl ControlChannel {
    pub async fn open(&mut self) -> U3vResult<()> {
        if self.iface.is_none() {
            // Claim interface (MaybeFuture)
            let iface = self
                .device
                .claim_interface(self.iface_info.iface_number)
                .await?;

            // Open bulk endpoints
            let tx_ep = iface.endpoint::<Bulk, Out>(self.iface_info.bulk_out_ep)?;
            let rx_ep = iface.endpoint::<Bulk, In>(self.iface_info.bulk_in_ep)?;

            // Wrap them into async IO adapters.
            let tx = tx_ep.writer(4096).with_num_transfers(4);
            let rx = rx_ep.reader(4096).with_num_transfers(4);

            self.tx = Some(tx);
            self.rx = Some(rx);
            self.iface = Some(iface);
        }

        Ok(())
    }

    pub async fn send(&mut self, buf: &[u8]) -> U3vResult<()> {
        use tokio::io::AsyncWriteExt;

        let tx = self.tx.as_mut().ok_or(U3vError::NoInterface)?;

        tx.write_all(buf).await?;
        // For U3V-style messages where a short/zero packet delimits the end:
        tx.flush_end_async().await?;
        Ok(())
    }

    pub async fn recv_exact(&mut self, buf: &mut [u8]) -> U3vResult<()> {
        use tokio::io::AsyncReadExt;

        let rx = self.rx.as_mut().ok_or(U3vError::NoInterface)?;
        rx.read_exact(buf).await?;
        Ok(())
    }

    /// Or if your protocol uses "short packet marks end of message":
    pub async fn recv_message(&mut self) -> U3vResult<Vec<u8>> {
        use tokio::io::AsyncReadExt;

        let rx = self.rx.as_mut().ok_or(U3vError::NoInterface)?;

        let mut reader = rx.until_short_packet();
        let mut v = Vec::new();
        reader.read_to_end(&mut v).await?;
        reader.consume_end()?;
        Ok(v)
    }

    pub async fn clear_halt(&mut self) -> U3vResult<()> {
        // If we never opened, nothing to clear.
        if self.tx.is_none() && self.rx.is_none() {
            return Ok(());
        }

        // OUT endpoint
        if let Some(tx) = self.tx.take() {
            // Take back the underlying Endpoint<_, Out>
            let mut ep = tx.into_inner();

            // Optionally: ep.cancel_all(); and/or drain completions if you really want
            ep.clear_halt().await?;

            // Re-wrap into EndpointWrite with the same settings
            let tx_wrapped = ep.writer(4096).with_num_transfers(4);
            self.tx = Some(tx_wrapped);
        }

        // IN endpoint
        if let Some(rx) = self.rx.take() {
            let mut ep = rx.into_inner();
            ep.clear_halt().await?;
            let rx_wrapped = ep.reader(4096).with_num_transfers(4);
            self.rx = Some(rx_wrapped);
        }

        Ok(())
    }

    pub(super) fn new(device: nusb::Device, iface_info: ControlIfaceInfo) -> Self {
        Self {
            device,
            iface_info,
            iface: None,
            tx: None,
            rx: None,
        }
    }
}
//
pub struct ReceiveChannel {
    pub(super) device: nusb::Device,
    pub iface_info: ReceiveIfaceInfo,
    pub iface: Option<Interface>,
}

impl ReceiveChannel {
    pub async fn open(&mut self) -> U3vResult<()> {
        if self.iface.is_none() {
            let iface = self
                .device
                .claim_interface(self.iface_info.iface_number)
                .await?;
            self.iface = Some(iface);
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if self.iface.is_some() {
            self.iface = None;
        }
    }

    #[must_use]
    pub fn is_opened(&self) -> bool {
        self.iface.is_some()
    }

    pub(super) fn new(device: nusb::Device, iface_info: ReceiveIfaceInfo) -> Self {
        Self {
            device,
            iface_info,
            iface: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ControlIfaceInfo {
    pub iface_number: u8,
    pub bulk_in_ep: u8,
    pub bulk_out_ep: u8,
}

#[derive(Clone, Debug)]
pub struct ReceiveIfaceInfo {
    pub iface_number: u8,
    pub bulk_in_ep: u8,
}
