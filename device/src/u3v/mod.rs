/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

pub mod protocol;
pub mod register_map;
pub mod prelude {
    pub use protocol::ack::ParseScd;
    pub use protocol::cmd::CommandScd;

    use super::protocol;
}

mod channel;
mod device;
mod device_builder;
mod device_info;

pub use channel::{ControlChannel, ReceiveChannel};
pub use device::Device;
pub use device_builder::enumerate_devices;
pub use device_info::{BusSpeed, DeviceInfo};

use std::borrow::Cow;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum U3vError {
    // NUsb error are actually std::io::Error's
    #[error("nusb error: {0}")]
    NUsb(#[from] nusb::Error),

    #[error("packet is broken: {0}")]
    InvalidPacket(Cow<'static, str>),

    #[error("device doesn't follow the specification")]
    InvalidDevice,
}

pub type Result<T> = std::result::Result<T, U3vError>;
