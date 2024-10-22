/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::u3v::{DeviceInfo, Result};

use super::channel::{ControlChannel, ControlIfaceInfo, ReceiveChannel, ReceiveIfaceInfo};

/// Entry point to the connected device.
/// This device itself doesn't communicate with the connected device but provide basic device
/// information and channels to communicate with the connected device. So it's valid to use
/// provided channels even after dropping this instance.
pub struct Device {
    device: nusb::Device,

    ctrl_iface_info: ControlIfaceInfo,
    event_iface_info: Option<ReceiveIfaceInfo>,
    stream_iface_info: Option<ReceiveIfaceInfo>,

    pub device_info: DeviceInfo,
}

impl Device {
    pub fn control_channel(&self) -> Result<ControlChannel> {
        Ok(ControlChannel::new(
            self.device.clone(),
            self.ctrl_iface_info.clone(),
        ))
    }

    pub fn event_channel(&self) -> Result<Option<ReceiveChannel>> {
        match &self.event_iface_info {
            Some(iface_info) => Ok(Some(ReceiveChannel::new(
                self.device.clone(),
                iface_info.clone(),
            ))),
            None => Ok(None),
        }
    }

    pub fn stream_channel(&self) -> Result<Option<ReceiveChannel>> {
        match &self.stream_iface_info {
            Some(iface_info) => Ok(Some(ReceiveChannel::new(
                self.device.clone(),
                iface_info.clone(),
            ))),
            None => Ok(None),
        }
    }

    #[must_use]
    pub fn device_info(&self) -> &DeviceInfo {
        &self.device_info
    }

    pub(super) fn new(
        device: nusb::Device,
        ctrl_iface_info: ControlIfaceInfo,
        event_iface_info: Option<ReceiveIfaceInfo>,
        stream_iface_info: Option<ReceiveIfaceInfo>,
        device_info: DeviceInfo,
    ) -> Self {
        let device = Self {
            device,
            ctrl_iface_info,
            event_iface_info,
            stream_iface_info,
            device_info,
        };

        log::info! {"{}: create device", device.log_name()};
        device
    }

    //TODO: We need logger.
    fn log_name(&self) -> String {
        format!(
            "{}-{}-{}",
            self.device_info.vendor_name,
            self.device_info.model_name,
            self.device_info.serial_number,
        )
    }
}

cfg_if::cfg_if! {
if #[cfg(target_os = "windows")] {
    use std::{
        sync::{Arc, Mutex},
        time,
    };

    pub(super) struct LibUsbDevice {
        pub(super) handle: LibUsbDeviceHandle,
    }
    impl LibUsbDevice {
        pub(super) fn open(&self) -> Result<LibUsbDeviceHandle> {
            Ok(self.handle.clone())
        }

        fn new(device: RusbDevice) -> Self {
            let handle = LibUsbDeviceHandle {
                device: Arc::new(Mutex::new(device)),
                handle: Arc::new(Mutex::new(None)),
            };

            Self { handle }
        }
    }

    #[derive(Clone)]
    pub(super) struct LibUsbDeviceHandle {
        device: Arc<Mutex<RusbDevice>>,
        pub(super) handle: Arc<Mutex<Option<RusbDeviceHandle>>>,
    }
    macro_rules! delegate {
        ($handle:expr, $method:ident($($args:ident),*)) => {
            if let Some(handle) = &mut *$handle {
                handle.$method($($args),*).map_err(Into::into)
            } else {
                Err(super::LibUsbError::Io.into())
            }
        }
    }
    impl LibUsbDeviceHandle {
        pub(super) fn claim_interface(&mut self, iface: u8) -> Result<()> {
            let mut handle = self.handle.lock().unwrap();
            if handle.is_none() {
                let device = self.device.lock().unwrap();
                *handle = device.open()?.into();
            }

            delegate!(handle, claim_interface(iface))
        }

        pub(super) fn release_interface(&mut self, iface: u8) -> Result<()> {
            let mut handle = self.handle.lock().unwrap();
            if let Some(handle) = &mut *handle {
                handle.release_interface(iface).map_err(Into::into)
            } else {
                Ok(())
            }
        }

        pub(super) fn read_bulk(
            &self,
            endpoint: u8,
            buf: &mut [u8],
            timeout: time::Duration,
        ) -> Result<usize> {
            let mut handle = self.handle.lock().unwrap();
            delegate!(handle, read_bulk(endpoint, buf, timeout))
        }

        pub(super) fn write_bulk(
            &self,
            endpoint: u8,
            buf: &[u8],
            timeout: time::Duration,
        ) -> Result<usize> {
            let mut handle = self.handle.lock().unwrap();
            delegate!(handle, write_bulk(endpoint, buf, timeout))
        }

        pub(super) fn clear_halt(&mut self, endpoint: u8) -> Result<()> {
            let mut handle = self.handle.lock().unwrap();
            delegate!(handle, clear_halt(endpoint))
        }

        pub(super) fn write_control(
            &self,
            request_type: u8,
            request: u8,
            value: u16,
            index: u16,
            buf: &[u8],
            timeout: time::Duration,
        ) -> Result<usize> {
            let mut handle = self.handle.lock().unwrap();
            delegate!(
                handle,
                write_control(request_type, request, value, index, buf, timeout)
            )
        }
    }

    fn get_device(device:RusbDevice) -> LibUsbDevice {
        LibUsbDevice::new(device)
    }
}}
