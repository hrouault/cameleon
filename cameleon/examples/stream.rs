/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This example describes how to start streaming and receive payloads.

use cameleon::{u3v::enumerate_cameras, CameleonError, CameleonResult};
use futures_lite::{pin, StreamExt};
use std::sync::mpsc;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> CameleonResult<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    // Enumerates cameras connected to the host.
    let mut cameras = enumerate_cameras().await?;

    if cameras.is_empty() {
        return Err(CameleonError::NoCamera);
    }

    let mut camera = cameras.pop().unwrap();

    // Open the camera.
    camera.open()?;
    // Load `GenApi` context.
    camera.load_context()?;

    // Start streaming. Can use buffered to add capacity to the stream
    let (reuse_tx, reuse_rx) = mpsc::channel::<Vec<u8>>();
    let stream = camera.start_streaming(reuse_rx)?.take(10);

    pin!(stream);
    while let Some(res) = stream.next().await {
        let payload = match res {
            Ok(payload) => payload,
            Err(e) => {
                warn!("payload receive error: {e}");
                continue;
            }
        };
        info!(
            "payload received! block_id: {:?}, timestamp: {:?}",
            payload.id(),
            payload.timestamp()
        );
        if let Some(image_info) = payload.image_info() {
            info!("{:?}\n", image_info);
        }

        payload.return_buffer(&reuse_tx);
    }
    camera.close()
}
