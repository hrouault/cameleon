/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::u3v::Result;
use futures_lite::future::block_on;
use nusb::{
    transfer::{ControlOut, ControlType, Recipient},
    Interface,
};
use std::time;

pub struct ControlChannel {
    pub(super) device: nusb::Device,
    pub iface_info: ControlIfaceInfo,
    pub iface: Option<Interface>,
}

impl ControlChannel {
    pub fn open(&mut self) -> Result<()> {
        if self.iface.is_none() {
            self.device.claim_interface(self.iface_info.iface_number)?;
        }

        Ok(())
    }
    //
    //     pub fn send(&self, buf: &[u8], timeout: time::Duration) -> Result<usize> {
    //         Ok(self
    //             .iface
    //             .bulk_out(self.iface_info.bulk_out_ep, buf, timeout)?)
    //     }
    //
    //     pub fn recv(&self, buf: &mut [u8], timeout: time::Duration) -> Result<usize> {
    //         Ok(self
    //             .iface
    //             .bulk_in(self.iface_info.bulk_in_ep, buf, timeout)?)
    //     }
    //
    //     pub fn set_halt(&self, timeout: time::Duration) -> Result<()> {
    //         set_halt(&self.device_handle, self.iface_info.bulk_in_ep, timeout)?;
    //         set_halt(&self.device_handle, self.iface_info.bulk_out_ep, timeout)?;
    //
    //         Ok(())
    //     }
    //
    //     pub fn clear_halt(&mut self) -> Result<()> {
    //         self.device_handle.clear_halt(self.iface_info.bulk_in_ep)?;
    //         self.device_handle.clear_halt(self.iface_info.bulk_out_ep)?;
    //         Ok(())
    //     }
    //
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
    pub fn open(&mut self) -> Result<()> {
        if self.iface.is_none() {
            self.device.claim_interface(self.iface_info.iface_number)?;
        }

        Ok(())
    }
    //
    //     pub fn close(&mut self) -> Result<()> {
    //         if self.is_opened() {
    //             self.device_handle
    //                 .release_interface(self.iface_info.iface_number)?;
    //         }
    //
    //         self.is_opened = false;
    //         Ok(())
    //     }
    //
    //     #[must_use]
    //     pub fn is_opened(&self) -> bool {
    //         self.is_opened
    //     }
    //
    //     pub fn recv(&self, buf: &mut [u8], timeout: time::Duration) -> Result<usize> {
    //         Ok(self
    //             .device_handle
    //             .read_bulk(self.iface_info.bulk_in_ep, buf, timeout)?)
    //     }
    //
    //     pub fn set_halt(&self, timeout: time::Duration) -> Result<()> {
    //         set_halt(&self.device_handle, self.iface_info.bulk_in_ep, timeout)?;
    //
    //         Ok(())
    //     }
    //
    //     pub fn clear_halt(&mut self) -> Result<()> {
    //         self.device_handle.clear_halt(self.iface_info.bulk_in_ep)?;
    //         Ok(())
    //     }
    //
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

// fn set_halt(handle: &nusb::Device, endpoint_number: u8, timeout: time::Duration) -> Result<()> {
//     let request = 0x03; // SET_FEATURE.
//     let value = 0x00; // ENDPOINT_HALT.
//     let buf = vec![]; // NO DATA.
//
//     let interface = handle.claim_interface(0).unwrap();
//     let result = block_on(interface.control_out(ControlOut {
//         control_type: ControlType::Vendor,
//         recipient: Recipient::Endpoint,
//         request,
//         value,
//         index: endpoint_number as u16,
//         data: &buf,
//     }))?;
//
//     Ok(())
// }
