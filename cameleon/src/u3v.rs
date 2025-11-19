/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This module provides low level API for U3V compatible devices.
//!
//! # Examples
//!
//! ```rust
//! use cameleon::Camera;
//! use cameleon::u3v;
//! use cameleon::genapi;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Enumerates cameras connected to the host.
//!     let mut cameras = u3v::enumerate_cameras().await.unwrap();
//!
//!     // If no camera is found, return.
//!     if cameras.is_empty() {
//!         return;
//!     }
//!
//!     let mut camera = cameras.pop().unwrap();
//!     // Opens the camera.
//!     camera.open();
//!
//!     let ctrl = &mut camera.ctrl;
//!     // Get Abrm.
//!     let abrm = ctrl.abrm().unwrap();
//!
//!     // Read serial number from ABRM.
//!     let serial_number = abrm.serial_number(ctrl).unwrap();
//!     println!("{}", serial_number);
//!
//!     // Check user defined name feature is supported.
//!     // If it is suppoted, read from and write to the register.
//!     let device_capability = abrm.device_capability().unwrap();
//!     if device_capability.is_user_defined_name_supported() {
//!         // Read from user defined name register.
//!         let user_defined_name = abrm.user_defined_name(ctrl).unwrap().unwrap();
//!         println!("{}", user_defined_name);
//!
//!         // Write new name to the register.
//!         abrm.set_user_defined_name(ctrl, "cameleon").unwrap();
//!     }
//! }
//! ```
#![allow(clippy::missing_panics_doc)]

pub mod control_handle;
pub mod register_map;
pub mod stream_handle;

pub use control_handle::{ControlHandle, SharedControlHandle};
pub use stream_handle::{StreamHandle, StreamParams};

pub use cameleon_device::u3v::DeviceInfo;

use cameleon_device::u3v;
use tracing::info;

use super::{genapi::DefaultGenApiCtxt, CameleonResult, Camera, CameraInfo};

/// Enumerate all U3V compatible cameras connected to the host.
///
/// # Examples
///
/// ```rust
/// use cameleon::Camera;
/// use cameleon::u3v;
/// use cameleon::genapi;
///
/// #[tokio::main]
/// async fn main() {
///     // Enumerate cameras connected to the host.
///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
/// }
/// ```
pub async fn enumerate_cameras() -> CameleonResult<Vec<Camera<ControlHandle, StreamHandle>>> {
    info!("Trying to enumerate cameras");
    let devices = u3v::enumerate_devices().await?;

    let mut cameras: Vec<Camera<ControlHandle, StreamHandle>> = Vec::with_capacity(devices.len());

    for dev in devices {
        info!("Trying to handle camera: {:?}", dev.device_info());
        let ctrl = ControlHandle::new(&dev)?;
        info!("Trying to get stream");
        let strm = if let Some(strm) = StreamHandle::new(&dev)? {
            strm
        } else {
            continue;
        };
        let ctxt = None;

        info!("Build device info");
        let dev_info = dev.device_info();
        let camera_info = CameraInfo {
            vendor_name: dev_info.vendor_name.clone(),
            model_name: dev_info.model_name.clone(),
            serial_number: dev_info.serial_number.clone(),
        };

        let camera: Camera<ControlHandle, StreamHandle, DefaultGenApiCtxt> =
            Camera::new(ctrl, strm, ctxt, camera_info);
        cameras.push(camera)
    }

    Ok(cameras)
}
