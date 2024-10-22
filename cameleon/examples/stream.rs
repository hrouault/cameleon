/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This example describes how to start streaming and receive payloads.

use cameleon::{u3v::enumerate_cameras, PayloadStream};
use futures_lite::future;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    // Enumerates cameras connected to the host.
    let mut cameras = enumerate_cameras().unwrap();

    if cameras.is_empty() {
        println!("no camera found!");
        return;
    }

    let mut camera = cameras.pop().unwrap();

    // Open the camera.
    camera.open().unwrap();
    // Load `GenApi` context.
    camera.load_context().unwrap();

    // Start streaming. Can use buffered to add capacity to the stream
    camera.start_streaming().unwrap();

    for _ in 0..10 {
        let payload = match future::block_on(camera.strm.next_payload()) {
            Ok(payload) => payload,
            Err(e) => {
                println!("payload receive error: {e}");
                continue;
            }
        };
        println!(
            "payload received! block_id: {:?}, timestamp: {:?}",
            payload.id(),
            payload.timestamp()
        );
        if let Some(image_info) = payload.image_info() {
            println!("{:?}\n", image_info);
        }

        // Send back payload to streaming loop to reuse the buffer.
        let _ = camera.strm.reuse_payload(payload.reuse_payload());
    }

    camera.close().ok();
}
