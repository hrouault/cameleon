/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

extern crate cameleon_device;

use cameleon_device::u3v::{
    enumerate_devices,
    prelude::*,
    protocol::{ack, cmd},
    register_map, Device,
};
use futures_lite::future::block_on;
use nusb::transfer::RequestBuffer;
use std::ffi::CStr;

fn main() {
    // Enumerate devices connected to the host.
    let devices: Vec<Device> = enumerate_devices().unwrap().into_iter().collect();

    if devices.is_empty() {
        println!("no device found");
        return;
    }

    let device = &devices[0];

    let request_id = 0;

    // Get control channel of the device.
    let mut control_channel = device.control_channel().unwrap();

    // Open the channel to allow communication with the device.
    control_channel.open().unwrap();

    // Get address and length of serial number register in ABRM.
    let (addr, len) = register_map::abrm::SERIAL_NUMBER;

    // Create ReadMem Command with request id.
    let command = cmd::ReadMem::new(addr, len).finalize(request_id);

    // Serialize the command.
    let mut serialized_command = vec![];
    command.serialize(&mut serialized_command).unwrap();

    //  Send read request to the device.
    block_on(control_channel.send(serialized_command).unwrap())
        .into_result()
        .unwrap();

    // Receive Acknowledge packet from the device.
    let serialized_ack = block_on(
        control_channel
            .recv(RequestBuffer::new(command.maximum_ack_len()))
            .unwrap(),
    )
    .into_result()
    .unwrap();

    // Parse Acknowledge packet.
    let ack = ack::AckPacket::parse(&serialized_ack).unwrap();

    // Check status and request_id.
    if !ack.status().is_success() || ack.request_id() != request_id {
        println!("Invalid acknowledge packet!");
        return;
    }

    // Parse SCD.
    let scd = ack.scd_as::<ack::ReadMem>().unwrap();

    let string_len = scd.data.iter().position(|c| *c == 0).unwrap();
    let serial_number = CStr::from_bytes_with_nul(&scd.data[..=string_len]).unwrap();

    println!(
        "Serial number received! {}",
        serial_number.to_str().unwrap()
    );
}
