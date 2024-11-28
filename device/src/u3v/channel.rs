/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::u3v::{U3vError, U3vResult};
use nusb::{
    transfer::{RequestBuffer, TransferFuture},
    Interface,
};

pub struct ControlChannel {
    pub(super) device: nusb::Device,
    pub iface_info: ControlIfaceInfo,
    pub iface: Option<Interface>,
}

impl ControlChannel {
    pub fn open(&mut self) -> U3vResult<()> {
        if self.iface.is_none() {
            self.iface = Some(self.device.claim_interface(self.iface_info.iface_number)?);
        }

        Ok(())
    }

    pub fn send(&self, buf: Vec<u8>) -> U3vResult<TransferFuture<Vec<u8>>> {
        if let Some(iface) = &self.iface {
            Ok(iface.bulk_out(self.iface_info.bulk_out_ep, buf))
        } else {
            Err(U3vError::NoInterface)
        }
    }

    pub fn recv(&self, buf: RequestBuffer) -> U3vResult<TransferFuture<RequestBuffer>> {
        if let Some(iface) = &self.iface {
            Ok(iface.bulk_in(self.iface_info.bulk_in_ep, buf))
        } else {
            Err(U3vError::NoInterface)
        }
    }

    pub fn clear_halt(&mut self) -> U3vResult<()> {
        if let Some(iface) = &self.iface {
            iface.clear_halt(self.iface_info.bulk_in_ep)?;
            iface.clear_halt(self.iface_info.bulk_out_ep)?;
        }
        Ok(())
    }

    pub(super) fn new(device: nusb::Device, iface_info: ControlIfaceInfo) -> Self {
        Self {
            device,
            iface_info,
            iface: None,
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
    pub fn open(&mut self) -> U3vResult<()> {
        if self.iface.is_none() {
            self.iface = Some(self.device.claim_interface(self.iface_info.iface_number)?);
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
