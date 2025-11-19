/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This module contains types that is the main entry types of the `Cameleon`.
//!
//! # Examples
//! ```rust
//! use cameleon::u3v;
//! use futures_lite::StreamExt;
//! use std::sync::mpsc;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Enumerates all cameras connected to the host.
//!     let mut cameras = u3v::enumerate_cameras().await.unwrap();
//!     if cameras.is_empty() {
//!         println!("no camera found");
//!         return;
//!     }
//!
//!     let mut camera = cameras.pop().unwrap();
//!
//!     // Opens the camera.
//!     camera.open().unwrap();
//!     // Loads `GenApi` context. This is necessary for streaming.
//!     camera.load_context().unwrap();
//!
//!     // Create a reuse channel for frame buffers.
//!     let (reuse_tx, reuse_rx) = mpsc::channel::<Vec<u8>>();
//!
//!     // Start streaming and only consume 10 payloads in this example.
//!     let mut stream = camera.start_streaming(reuse_rx).unwrap().take(10);
//!
//!     while let Some(res) = stream.next().await {
//!         match res {
//!             Ok(payload) => {
//!                 println!(
//!                     "payload received! block_id: {:?}, timestamp: {:?}",
//!                     payload.id(),
//!                     payload.timestamp()
//!                 );
//!
//!                 if let Some(image_info) = payload.image_info() {
//!                     println!("{:?}\n", image_info);
//!                     if let Some(image) = payload.image() {
//!                         // do something with the image bytes...
//!                         let _ = image;
//!                     }
//!                 }
//!
//!                 // Send back payload buffer to streaming loop to reuse it. This is optional.
//!                 payload.return_buffer(&reuse_tx);
//!             }
//!             Err(_err) => {
//!                 // handle or log the error as needed
//!                 continue;
//!             }
//!         }
//!     }
//!
//!     // Closes the camera.
//!     camera.close().unwrap();
//! }
//! ```

use super::{
    genapi::{DefaultGenApiCtxt, FromXml, GenApiCtxt, ParamsCtxt},
    CameleonError, CameleonResult, ControlResult, StreamResult,
};
use crate::u3v::stream_handle::{PayloadGenerator, PayloadStream};
use auto_impl::auto_impl;
use std::{fs, sync::mpsc::Receiver};
use tracing::info;

/// Provides easy-to-use access to a `GenICam` compatible camera.
///
/// # Examples
/// ```rust
/// use cameleon::u3v;
/// use futures_lite::StreamExt;
/// use std::sync::mpsc;
///
/// #[tokio::main]
/// # async fn main() {
///     // Enumerates all cameras connected to the host.
///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
///     if cameras.is_empty() {
///         println!("no camera found");
///         return;
///     }
///     let mut camera = cameras.pop().unwrap();
///
///     // Opens the camera.
///     camera.open().unwrap();
///     // Loads `GenApi` context. This is necessary for streaming.
///     camera.load_context().unwrap();
///
///     // Create a reuse channel for frame buffers.
///     let (reuse_tx, reuse_rx) = mpsc::channel::<Vec<u8>>();
///
///     // Start streaming and only consume 10 payloads in this example.
///     let mut stream = camera.start_streaming(reuse_rx).unwrap().take(10);
///
///     while let Some(item) = stream.next().await {
///         match item {
///             Ok(payload) => {
///                 println!(
///                     "payload received! block_id: {:?}, timestamp: {:?}",
///                     payload.id(),
///                     payload.timestamp()
///                 );
///                 if let Some(image_info) = payload.image_info() {
///                     println!("{:?}\n", image_info);
///                     let image = payload.image();
///                     // do something with the image.
///                     // ...
///                 }
///
///                 // Send back payload to streaming loop to reuse the buffer. This is optional.
///                 payload.return_buffer(&reuse_tx);
///             }
///             Err(_err) => {
///                 continue;
///             }
///         }
///     }
///
///     // Closes the camera.
///     camera.close().unwrap();
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Camera<Ctrl, Strm, Ctxt = DefaultGenApiCtxt> {
    /// Device control handle of the camera.
    pub ctrl: Ctrl,
    /// Payload stream handle of the camera.
    pub strm: Strm,
    /// `GenApi context` of the camera.
    pub ctxt: Option<Ctxt>,
    /// Information of the camera.
    info: CameraInfo,
}

macro_rules! expect_node {
    ($ctxt:expr, $name:expr, $as_type:ident) => {{
        let err_msg = std::concat!("missing ", $name);
        let err_msg2 = std::concat!($name, " has invalid interface");
        $ctxt
            .node($name)
            .ok_or_else(|| CameleonError::InvalidGenApiXml(err_msg.into()))?
            .$as_type($ctxt)
            .ok_or_else(|| CameleonError::InvalidGenApiXml(err_msg2.into()))?
    }};
}

impl<Ctrl, Strm, Ctxt> Camera<Ctrl, Strm, Ctxt> {
    /// Opens the camera. Ensure calling this method before starting to use the camera.  
    ///
    /// See also [`close`](Self::close) which must be called when an opened camera is no more needed.
    ///
    /// # Examples
    /// ```rust
    /// use cameleon::u3v;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     if cameras.is_empty() {
    ///         return;
    ///     }
    ///     let mut camera = cameras.pop().unwrap();
    ///
    ///     // Opens the camera before using it.
    ///     camera.open().unwrap();
    ///     // .. Do something with camera.
    ///     // Closes the camera after using it.
    ///     camera.close().unwrap();
    /// }
    /// ```
    #[tracing::instrument(skip(self),
                          level = "info",
                          fields(camera = ?self.info()))]
    pub fn open(&mut self) -> CameleonResult<()>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
    {
        info!("try opening the device");
        self.ctrl.open()?;
        self.strm.open()?;
        info!("opened the device successfully");
        Ok(())
    }

    /// Closes the camera.  
    ///
    /// Make sure to call this method before the camera is dropped.
    /// To keep flexibility, this method is NOT automatically called when `Camera::drop` is called.
    ///
    /// # Examples
    /// ```rust
    /// use cameleon::u3v;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     if cameras.is_empty() {
    ///         return;
    ///     }
    ///     let mut camera = cameras.pop().unwrap();
    ///
    ///     // Opens the camera before using it.
    ///     camera.open().unwrap();
    ///     // .. Do something with camera.
    ///     // Closes the camera after using it.
    ///     camera.close().unwrap();
    /// }
    /// ```
    #[tracing::instrument(skip(self),
                          level = "info",
                          fields(camera = ?self.info()))]
    pub fn close(&mut self) -> CameleonResult<()>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
        Ctxt: GenApiCtxt,
    {
        info!("try closing the device");
        self.stop_streaming()?;
        if let Some(ctxt) = &mut self.ctxt {
            ctxt.clear_cache()
        }
        info!("closed the device successfully");
        Ok(())
    }

    /// Loads `GenApi` xml from the device and builds the context, then returns the `GenApi` xml
    /// string.  
    ///
    /// Once the context has been built, the string itself is no longer needed. Therefore, you can
    /// drop the returned string at any time.
    ///
    /// # Examples
    /// ```rust
    /// use cameleon::u3v;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Enumerates all cameras connected to the host.
    ///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     if cameras.is_empty() {
    ///         return;
    ///     }
    ///     let mut camera = cameras.pop().unwrap();
    ///
    ///     // Opens the camera before using it.
    ///     camera.open().unwrap();
    ///
    ///     // Loads context. This enables you to edit parameters of the camera and start payload streaming.
    ///     camera.load_context().unwrap();
    ///
    ///     // Closes the camera.
    ///     camera.close().unwrap();
    /// }
    /// ```
    pub fn load_context(&mut self) -> CameleonResult<String>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
        Ctxt: GenApiCtxt + FromXml,
    {
        let xml = self.ctrl.genapi()?;
        fs::write("camera.xml", &xml).expect("Unable to write file");
        self.ctxt = Some(Ctxt::from_xml(&xml)?);
        Ok(xml)
    }

    /// Starts streaming and returns the receiver for the `Payload`.
    ///
    /// Make sure to load `GenApi` context before calling this method.
    /// See [`load_context`](Self::load_context) and [`set_context`](Self::set_context) how to configure `GenApi` context.
    ///
    /// NOTE: This method doesn't change `AcquisitionMode` which defined in `GenICam SFNC`.  
    /// We recommend you to set the node to `Continuous` if you don't know which mode is the best.
    ///
    /// See the `GenICam SFNC` specification for more details.
    ///
    /// # Examples
    /// ```rust
    /// # use cameleon::u3v;
    /// use futures_lite::StreamExt;
    /// use std::sync::mpsc;
    ///
    ///  #[tokio::main]
    ///  async fn main() {
    ///     # let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     # if cameras.is_empty() {
    ///     #     return;
    ///     # }
    ///     # let mut camera = cameras.pop().unwrap();
    ///     camera.open().unwrap();
    ///     camera.load_context().unwrap();
    ///     // Create a reuse channel for frame buffers.
    ///     let (reuse_tx, reuse_rx) = mpsc::channel::<Vec<u8>>();
    ///
    ///     // Start streaming.
    ///     let mut stream = camera.start_streaming(reuse_rx).unwrap();
    ///
    ///     // The streamed payloads can be received like this:
    ///     if let Some(Ok(payload)) = stream.next().await {
    ///         // Use the payload (image, metadata, etc.).
    ///         if let Some(image_info) = payload.image_info() {
    ///             println!("{:?}", image_info);
    ///         }
    ///
    ///         // Optionally send the buffer back for reuse.
    ///         payload.return_buffer(&reuse_tx);
    ///     }
    ///
    ///     // Closes the camera.
    ///     camera.close().unwrap();
    /// }
    /// ```
    ///
    /// # Arguments
    /// * `cap` - A capacity of the paylaod receiver, the sender will stop to send a payload when it
    /// gets full.
    ///
    ///
    /// # Panics
    /// If `cap` is zero, this method will panic.
    #[tracing::instrument(skip(self, payload_rx),
                          level = "info",
                          fields(camera = ?self.info()))]
    pub fn start_streaming(
        &mut self,
        payload_rx: Receiver<Vec<u8>>,
    ) -> CameleonResult<PayloadStream>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
        Ctxt: GenApiCtxt,
    {
        info!("try starting streaming");

        // Enable streaimng.
        self.ctrl.enable_streaming()?;
        let mut ctxt = self.params_ctxt()?;
        expect_node!(&ctxt, "TLParamsLocked", as_integer).set_value(&mut ctxt, 1)?;
        expect_node!(&ctxt, "AcquisitionStart", as_command).execute(&mut ctxt)?;
        Ok(self.strm.start_streaming(&mut self.ctrl, payload_rx)?)
    }

    #[tracing::instrument(skip(self, payload_rx),
                          level = "info",
                          fields(camera = ?self.info()))]
    pub fn start_generator(
        &mut self,
        payload_rx: Receiver<Vec<u8>>,
    ) -> CameleonResult<PayloadGenerator>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
        Ctxt: GenApiCtxt,
    {
        info!("try starting streaming");

        // Enable streaimng.
        self.ctrl.enable_streaming()?;
        let mut ctxt = self.params_ctxt()?;
        expect_node!(&ctxt, "TLParamsLocked", as_integer).set_value(&mut ctxt, 1)?;
        expect_node!(&ctxt, "AcquisitionStart", as_command).execute(&mut ctxt)?;
        Ok(self.strm.start_generator(&mut self.ctrl, payload_rx)?)
    }

    /// Stops the streaming.
    ///
    /// The receiver returned from the previous [`Self::start_streaming`]
    /// call will be invalidated.
    ///
    /// This method is automatically called in [`close`](Self::close), so no need to call
    /// explicitly when you close the camera.
    ///
    /// # Examples
    /// ```rust
    /// # use cameleon::u3v;
    /// # use std::sync::mpsc;
    /// # #[tokio::main]
    /// # async fn main() {
    /// # let mut cameras = u3v::enumerate_cameras().await.unwrap();
    /// # if cameras.is_empty() {
    /// #     return;
    /// # }
    /// # let mut camera = cameras.pop().unwrap();
    /// camera.open().unwrap();
    /// // Loads `GenApi` context. This is necessary for streaming.
    /// camera.load_context().unwrap();
    ///
    /// // Create a reuse channel for frame buffers.
    /// let (_reuse_tx, reuse_rx) = mpsc::channel::<Vec<u8>>();
    ///
    /// // Start streaming.
    /// let _stream = camera.start_streaming(reuse_rx).unwrap();
    ///
    /// // Stop streaming.
    /// camera.stop_streaming().unwrap();
    ///
    /// # camera.close().unwrap();
    /// # }
    /// ```
    #[tracing::instrument(skip(self),
                          level = "info",
                          fields(camera = ?self.info()))]
    pub fn stop_streaming(&mut self) -> CameleonResult<()>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
        Ctxt: GenApiCtxt,
    {
        info!("try stopping streaming");
        // Disable streaming.
        let mut ctxt = self.params_ctxt()?;
        expect_node!(&ctxt, "AcquisitionStop", as_command).execute(&mut ctxt)?;
        expect_node!(&ctxt, "TLParamsLocked", as_integer).set_value(&mut ctxt, 0)?;
        self.ctrl.disable_streaming()?;

        info!("stop streaming successfully");
        Ok(())
    }

    /// Returns the context of the camera params.
    ///
    /// Make sure to load `GenApi` context before calling this method.
    /// See [`load_context`](Self::load_context) and [`set_context`](Self::set_context) how to configure `GenApi` context.
    ///
    /// # Examples
    /// ```rust
    /// use cameleon::u3v;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Enumerates all cameras connected to the host.
    ///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     if cameras.is_empty() {
    ///         return;
    ///     }
    ///     let mut camera = cameras.pop().unwrap();
    ///
    ///     camera.open().unwrap();
    ///     camera.load_context().unwrap();
    ///
    ///     // Get params context.
    ///     let mut params_ctxt = camera.params_ctxt().unwrap();
    ///
    ///     // Get `Gain` node of `GenApi`.
    ///     // `GenApi SFNC` defines that `Gain` node should have `IFloat` interface,
    ///     // so this conversion would succeed if the camera follows that.
    ///     // Some vendors may define `Gain` node as `IInteger`, in that case, use
    ///     // `as_integer(&params_ctxt)` instead of `as_float(&params_ctxt)`.
    ///     let gain_node = params_ctxt
    ///         .node("Gain").unwrap()
    ///         .as_float(&params_ctxt).unwrap();
    ///
    ///     // Get the current value of `Gain`.
    ///     if gain_node.is_readable(&mut params_ctxt).unwrap() {
    ///         let value = gain_node.value(&mut params_ctxt).unwrap();
    ///         println!("{}", value);
    ///     }
    ///
    ///     // Set `0.1` to `Gain`.
    ///     if gain_node.is_writable(&mut params_ctxt).unwrap() {
    ///         gain_node.set_value(&mut params_ctxt, 0.1).unwrap();
    ///     }
    ///
    ///     camera.close().unwrap();
    /// }
    /// ```
    pub fn params_ctxt(&mut self) -> CameleonResult<ParamsCtxt<&mut Ctrl, &mut Ctxt>>
    where
        Ctrl: DeviceControl,
        Strm: StreamInterface,
        Ctxt: GenApiCtxt,
    {
        if let Some(ctxt) = self.ctxt.as_mut() {
            Ok(ParamsCtxt {
                ctrl: &mut self.ctrl,
                ctxt,
            })
        } else {
            Err(CameleonError::GenApiContextMissing)
        }
    }

    /// Returns basic information of the camera.
    ///
    /// This information can be obtained without calling [`Self::open`].
    ///
    /// # Examples
    /// ```rust
    /// use cameleon::u3v;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     if cameras.is_empty() {
    ///         return;
    ///     }
    ///     let camera = cameras.pop().unwrap();
    ///
    ///     let info = camera.info();
    ///     println!("{} {} {}", info.vendor_name, info.model_name, info.serial_number);
    /// }
    /// ```
    pub fn info(&self) -> &CameraInfo {
        &self.info
    }

    /// Constructs a camera.
    pub fn new(ctrl: Ctrl, strm: Strm, ctxt: Option<Ctxt>, info: CameraInfo) -> Self {
        Self {
            ctrl,
            strm,
            ctxt,
            info,
        }
    }

    /// Converts internal types.
    ///
    /// This method works same as `std::convert::From`, just hack to avoid
    /// `E0119`.
    pub fn convert_from<Ctrl2, Strm2, Ctxt2>(from: Camera<Ctrl2, Strm2, Ctxt2>) -> Self
    where
        Ctrl: From<Ctrl2>,
        Strm: From<Strm2>,
        Ctxt: From<Ctxt2>,
    {
        Camera::new(
            from.ctrl.into(),
            from.strm.into(),
            from.ctxt.map(|ctxt| ctxt.into()),
            from.info,
        )
    }

    /// Converts internal types. This method work same as `std::convert::Into`, just hack to avoid
    /// `E0119`.
    ///
    /// # Examples
    /// ```rust
    /// use cameleon::u3v;
    /// use cameleon::{DeviceControl, StreamInterface, Camera};
    /// use cameleon::genapi::NoCacheGenApiCtxt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // Enumerate cameras.
    ///     let mut cameras = u3v::enumerate_cameras().await.unwrap();
    ///     if cameras.is_empty() {
    ///         return;
    ///     }
    ///     let camera = cameras.pop().unwrap();
    ///
    ///     // Convert into `Camera<Box<dyn DeviceControl>, Box<dyn StreamInterface>, NoCacheGenApiCtxt>`.
    ///     let dyn_camera: Camera<
    ///         Box<dyn DeviceControl>,
    ///         Box<dyn StreamInterface>,
    ///         NoCacheGenApiCtxt,
    ///     > = camera.convert_into();
    /// }
    /// ```
    pub fn convert_into<Ctrl2, Strm2, Ctxt2>(self) -> Camera<Ctrl2, Strm2, Ctxt2>
    where
        Ctrl: Into<Ctrl2>,
        Strm: Into<Strm2>,
        Ctxt: Into<Ctxt2>,
    {
        Camera::new(
            self.ctrl.into(),
            self.strm.into(),
            self.ctxt.map(|ctxt| ctxt.into()),
            self.info,
        )
    }

    /// Set a context to the camera. It's recommended to use [`Self::load_context`] instead if `Self::Ctxt`
    /// implements [`FromXml`] trait.
    pub fn set_context<Ctxt2>(self, ctxt: Ctxt2) -> Camera<Ctrl, Strm, Ctxt2> {
        Camera {
            ctrl: self.ctrl,
            strm: self.strm,
            ctxt: Some(ctxt),
            info: self.info,
        }
    }
}

/// Information of the camera.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CameraInfo {
    /// Vendor name of the camera.
    pub vendor_name: String,
    /// Model name of the camera.
    pub model_name: String,
    ///Serial number of the camera.
    pub serial_number: String,
}

/// This trait provides operations on the device's memory.
#[auto_impl(&mut, Box)]
pub trait DeviceControl {
    /// Opens the handle.
    fn open(&mut self) -> ControlResult<()>;

    /// Returns `true` if device is already opened.
    fn is_opened(&self) -> bool;

    /// Reads data from the device's memory.
    ///
    /// Reads length is same as `buf.len()`.
    fn read(&mut self, address: u64, buf: &mut [u8]) -> ControlResult<()>;

    /// Writes data to the device's memory.
    fn write(&mut self, address: u64, data: &[u8]) -> ControlResult<()>;

    /// Returns `GenICam` xml string.
    fn genapi(&mut self) -> ControlResult<String>;

    /// Enables streaming.
    fn enable_streaming(&mut self) -> ControlResult<()>;

    /// Disables streaming.
    fn disable_streaming(&mut self) -> ControlResult<()>;
}

/// This trait provides streaming capability.
// #[auto_impl(&mut, Box)]
pub trait StreamInterface {
    /// Opens the handle.
    fn open(&mut self) -> StreamResult<()>;

    /// Starts streaming.
    fn start_streaming(
        &self,
        ctrl: &mut dyn DeviceControl,
        payload_rx: Receiver<Vec<u8>>,
    ) -> StreamResult<PayloadStream>;

    /// Starts the generator (same but without the stream interface)
    /// The output Payload generator does not implement the Stream trait. One uses the next_payload
    /// method of PayloadGenerator to asynchronously get the next picture.
    fn start_generator(
        &self,
        ctrl: &mut dyn DeviceControl,
        payload_rx: Receiver<Vec<u8>>,
    ) -> StreamResult<PayloadGenerator>;
}
