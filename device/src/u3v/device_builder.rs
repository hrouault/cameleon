/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::{
    channel::{ControlIfaceInfo, ReceiveIfaceInfo},
    device::Device,
};
use crate::u3v::{BusSpeed, DeviceInfo, Result, U3vError};
use log::debug;
use nusb::{
    descriptors::{self, language_id::US_ENGLISH, Descriptor},
    transfer::{Direction, EndpointType},
};
use semver::Version;
use std::time::Duration;

const MISCELLANEOUS_CLASS: u8 = 0xEF;

const DEVICE_SUBCLASS: u8 = 0x02;
const DEVICE_PROTOCOL: u8 = 0x01;

const IAD_DESC_TYPE: u8 = 0x0B;
const IAD_FUNCTION_PROTOCOL: u8 = 0x00;

const USB3V_SUBCLASS: u8 = 0x05;

pub fn enumerate_devices() -> Result<Vec<Device>> {
    let builders = nusb::list_devices()?.filter_map(|di| DeviceBuilder::new(di).ok().flatten());
    Ok(builders
        .filter_map(|builder| builder.build().ok())
        .collect())
}

struct DeviceBuilder {
    di: nusb::DeviceInfo,
    device: nusb::Device,
    u3v_iad: Iad,
    // config_desc: descriptors::Configuration,
}

impl DeviceBuilder {
    fn new(di: nusb::DeviceInfo) -> Result<Option<Self>> {
        if di.class() == MISCELLANEOUS_CLASS
            && di.subclass() == DEVICE_SUBCLASS
            && di.protocol() == DEVICE_PROTOCOL
        {
            let device = di.open()?;
            if let Some((iad, _conf_desc)) = Self::find_u3v_iad(&device)? {
                return Ok(Some(Self {
                    di,
                    device,
                    u3v_iad: iad,
                    // config_desc: conf_desc,
                }));
            }
        }
        Ok(None)
    }

    fn build(self) -> Result<Device> {
        // Skip interfaces while control interface is appeared.
        let mut interfaces = self
            .di
            .interfaces()
            .skip_while(|iface| iface.interface_number() != self.u3v_iad.first_interface);

        // Retrieve control interface information.
        let ctrl_iface_info = interfaces.next().ok_or(U3vError::InvalidDevice)?;
        debug!("Control interface info: {:?}", ctrl_iface_info);
        let ctrl_iface = self
            .device
            .claim_interface(ctrl_iface_info.interface_number())?;

        let ctrl_iface_info = ControlIfaceInfo::new(&ctrl_iface)?;

        // Retrieve device information.
        // This information is embedded next to control interface descriptor.
        let ctrl_iface_desc = ctrl_iface
            .descriptors()
            .next()
            .ok_or(U3vError::InvalidDevice)?;
        let iface_desc = ctrl_iface_desc.descriptors();
        let device_info = iface_desc
            .filter_map(|iface| DeviceInfoDescriptor::from_desc(&iface).ok())
            .next()
            .unwrap()
            .interpret(&self.device)?;

        // Retrieve event and stream interface information if exists.
        let mut receive_ifaces: Vec<(ReceiveIfaceInfo, ReceiveIfaceKind)> = interfaces
            .filter_map(|iface| {
                ReceiveIfaceInfo::new(
                    &self
                        .device
                        .claim_interface(iface.interface_number())
                        .unwrap(),
                )
            })
            .collect();

        if receive_ifaces.len() > 2 {
            return Err(U3vError::InvalidDevice);
        }

        let (event_iface, stream_iface) = match receive_ifaces.pop() {
            Some((event_iface, ReceiveIfaceKind::Event)) => match receive_ifaces.pop() {
                Some((stream_iface, ReceiveIfaceKind::Stream)) => {
                    (Some(event_iface), Some(stream_iface))
                }
                None => (Some(event_iface), None),
                Some(_) => return Err(U3vError::InvalidDevice),
            },
            Some((stream_iface, ReceiveIfaceKind::Stream)) => match receive_ifaces.pop() {
                Some((event_iface, ReceiveIfaceKind::Event)) => {
                    (Some(event_iface), Some(stream_iface))
                }
                None => (None, Some(stream_iface)),
                Some(_) => return Err(U3vError::InvalidDevice),
            },
            None => (None, None),
        };

        Ok(Device::new(
            self.device,
            ctrl_iface_info,
            event_iface,
            stream_iface,
            device_info,
        ))
    }

    fn find_u3v_iad(device: &nusb::Device) -> Result<Option<(Iad, descriptors::Configuration)>> {
        for conf in device.configurations() {
            if let Some(u3v_iad) = Self::find_u3v_iad_in_config(device, &conf) {
                return Ok(Some((u3v_iad, conf)));
            }
        }

        Ok(None)
    }

    fn find_u3v_iad_in_config(
        device: &nusb::Device,
        conf: &descriptors::Configuration,
    ) -> Option<Iad> {
        for desc in conf.descriptors() {
            if let Some(iad) = Iad::from_desc(desc) {
                if Self::is_u3v_iad(&iad) {
                    return Some(iad);
                }
            }
        }

        for iface_ind in conf.interfaces() {
            let iface = device
                .claim_interface(iface_ind.interface_number())
                .unwrap();
            for if_desc in iface.descriptors() {
                if let Some(u3v_iad) = Self::find_u3v_iad_in_ifce(&if_desc) {
                    return Some(u3v_iad);
                }
            }
        }

        None
    }

    fn find_u3v_iad_in_ifce(ifce: &descriptors::InterfaceAltSetting) -> Option<Iad> {
        for desc in ifce.descriptors() {
            if let Some(iad) = Iad::from_desc(desc) {
                if Self::is_u3v_iad(&iad) {
                    return Some(iad);
                }
            }
        }

        for ep_desc in ifce.endpoints() {
            if let Some(u3v_iad) = Self::find_u3v_iad_in_ep(&ep_desc) {
                return Some(u3v_iad);
            }
        }

        None
    }

    fn find_u3v_iad_in_ep(ep_desc: &descriptors::Endpoint) -> Option<Iad> {
        for desc in ep_desc.descriptors() {
            if let Some(iad) = Iad::from_desc(desc) {
                if Self::is_u3v_iad(&iad) {
                    return Some(iad);
                }
            }
        }

        None
    }

    fn is_u3v_iad(iad: &Iad) -> bool {
        iad.function_class == MISCELLANEOUS_CLASS
            && iad.function_subclass == USB3V_SUBCLASS
            && iad.function_protocol == IAD_FUNCTION_PROTOCOL
    }
}

/// Interface Association Descriptor.
#[allow(unused)]
#[derive(Debug)]
struct Iad {
    length: u8,
    descriptor_type: u8,
    first_interface: u8,
    interface_count: u8,
    function_class: u8,
    function_subclass: u8,
    function_protocol: u8,
    function: u8,
}

impl Iad {
    fn from_desc(desc: Descriptor) -> Option<Self> {
        let desc_length = desc.descriptor_len();
        if desc_length == 0 {
            return None;
        }
        let descriptor_type = desc.descriptor_type();
        if descriptor_type != IAD_DESC_TYPE {
            return None;
        }

        let first_interface = desc[2];
        let interface_count = desc[3];
        let function_class = desc[4];
        let function_subclass = desc[5];
        let function_protocol = desc[6];
        let function = desc[7];
        Some(Self {
            length: desc_length as u8,
            descriptor_type,
            first_interface,
            interface_count,
            function_class,
            function_subclass,
            function_protocol,
            function,
        })
    }
}

struct DeviceInfoDescriptor {
    #[allow(unused)]
    length: u8,
    #[allow(unused)]
    descriptor_type: u8,
    #[allow(unused)]
    descriptor_subtype: u8,
    gencp_version_major: u16,
    gencp_version_minor: u16,
    u3v_version_major: u16,
    u3v_version_minor: u16,
    guid_idx: u8,
    vendor_name_idx: u8,
    model_name_idx: u8,
    family_name_idx: u8,
    device_version_idx: u8,
    manufacturer_info_idx: u8,
    serial_number_idx: u8,
    user_defined_name_idx: u8,
    supported_speed_mask: u8,
}

impl DeviceInfoDescriptor {
    const MINIMUM_DESC_LENGTH: u8 = 20;
    const DESCRIPTOR_TYPE: u8 = 0x24;
    const DESCRIPTOR_SUBTYPE: u8 = 0x1;

    fn from_desc(desc: &Descriptor) -> Result<Self> {
        let desc_length = desc.descriptor_len();
        let descriptor_type = desc.descriptor_type();
        let descriptor_subtype = desc[2];

        if desc_length < Self::MINIMUM_DESC_LENGTH as usize
            || descriptor_type != Self::DESCRIPTOR_TYPE
            || descriptor_subtype != Self::DESCRIPTOR_SUBTYPE
        {
            return Err(U3vError::InvalidDevice);
        }

        let gencp_version_minor = u16::from_le_bytes(desc[3..5].try_into().unwrap());
        let gencp_version_major = u16::from_le_bytes(desc[5..7].try_into().unwrap());
        let u3v_version_minor = u16::from_le_bytes(desc[7..9].try_into().unwrap());
        let u3v_version_major = u16::from_le_bytes(desc[9..11].try_into().unwrap());
        let guid_idx = desc[11];
        let vendor_name_idx = desc[12];
        let model_name_idx = desc[13];
        let family_name_idx = desc[14];
        let device_version_idx = desc[15];
        let manufacturer_info_idx = desc[16];
        let serial_number_idx = desc[17];
        let user_defined_name_idx = desc[18];
        let supported_speed_mask = desc[19];

        Ok(Self {
            length: desc_length as u8,
            descriptor_type,
            descriptor_subtype,
            gencp_version_major,
            gencp_version_minor,
            u3v_version_major,
            u3v_version_minor,
            guid_idx,
            vendor_name_idx,
            model_name_idx,
            family_name_idx,
            device_version_idx,
            manufacturer_info_idx,
            serial_number_idx,
            user_defined_name_idx,
            supported_speed_mask,
        })
    }

    fn interpret(&self, channel: &nusb::Device) -> Result<DeviceInfo> {
        let gencp_version = Version::new(
            self.gencp_version_major.into(),
            self.gencp_version_minor.into(),
            0,
        );

        let u3v_version = Version::new(
            self.u3v_version_major.into(),
            self.u3v_version_minor.into(),
            0,
        );

        let guid =
            channel.get_string_descriptor(self.guid_idx, US_ENGLISH, Duration::from_millis(100))?;
        let vendor_name = channel.get_string_descriptor(
            self.vendor_name_idx,
            US_ENGLISH,
            Duration::from_millis(100),
        )?;
        let model_name = channel.get_string_descriptor(
            self.model_name_idx,
            US_ENGLISH,
            Duration::from_millis(100),
        )?;
        let family_name = if self.family_name_idx == 0 {
            None
        } else {
            Some(channel.get_string_descriptor(
                self.family_name_idx,
                US_ENGLISH,
                Duration::from_millis(100),
            )?)
        };

        let device_version = channel.get_string_descriptor(
            self.device_version_idx,
            US_ENGLISH,
            Duration::from_millis(100),
        )?;
        let manufacturer_info = channel.get_string_descriptor(
            self.manufacturer_info_idx,
            US_ENGLISH,
            Duration::from_millis(100),
        )?;
        let serial_number = channel.get_string_descriptor(
            self.serial_number_idx,
            US_ENGLISH,
            Duration::from_millis(100),
        )?;
        let user_defined_name = if self.user_defined_name_idx == 0 {
            None
        } else {
            Some(channel.get_string_descriptor(
                self.user_defined_name_idx,
                US_ENGLISH,
                Duration::from_millis(100),
            )?)
        };
        let supported_speed = if self.supported_speed_mask >> 4_i32 & 0b1 == 1 {
            BusSpeed::SuperSpeedPlus
        } else if self.supported_speed_mask >> 3_i32 & 0b1 == 1 {
            BusSpeed::SuperSpeed
        } else if self.supported_speed_mask >> 2_i32 & 0b1 == 1 {
            BusSpeed::HighSpeed
        } else if self.supported_speed_mask >> 1_i32 & 0b1 == 1 {
            BusSpeed::FullSpeed
        } else if self.supported_speed_mask & 0b1 == 1 {
            BusSpeed::LowSpeed
        } else {
            return Err(U3vError::InvalidDevice);
        };

        Ok(DeviceInfo {
            gencp_version,
            u3v_version,
            guid,
            vendor_name,
            model_name,
            family_name,
            device_version,
            manufacturer_info,
            serial_number,
            user_defined_name,
            supported_speed,
        })
    }
}

impl ControlIfaceInfo {
    const CONTROL_IFACE_PROTOCOL: u8 = 0x00;

    fn new(iface: &nusb::Interface) -> Result<Self> {
        let iface_number = iface.interface_number();
        let iface_desc = iface.descriptors().next().ok_or(U3vError::InvalidDevice)?;

        if iface_desc.class() != MISCELLANEOUS_CLASS
            || iface_desc.subclass() != USB3V_SUBCLASS
            || iface_desc.protocol() != Self::CONTROL_IFACE_PROTOCOL
        {
            return Err(U3vError::InvalidDevice);
        }

        let eps: Vec<descriptors::Endpoint> = iface_desc.endpoints().collect();
        if eps.len() != 2 {
            return Err(U3vError::InvalidDevice);
        }
        let ep_in = eps
            .iter()
            .find(|ep| ep.direction() == Direction::In)
            .ok_or(U3vError::InvalidDevice)?;
        let ep_out = eps
            .iter()
            .find(|ep| ep.direction() == Direction::Out)
            .ok_or(U3vError::InvalidDevice)?;
        if ep_in.transfer_type() != EndpointType::Bulk
            || ep_out.transfer_type() != EndpointType::Bulk
        {
            return Err(U3vError::InvalidDevice);
        }

        Ok(Self {
            iface_number,
            bulk_in_ep: ep_in.address(),
            bulk_out_ep: ep_out.address(),
        })
    }
}

impl ReceiveIfaceInfo {
    const EVENT_IFACE_PROTOCOL: u8 = 0x01;
    const STREAM_IFACE_PROTOCOL: u8 = 0x02;

    fn new(iface: &nusb::Interface) -> Option<(Self, ReceiveIfaceKind)> {
        let iface_number = iface.interface_number();
        for desc in iface.descriptors() {
            if desc.alternate_setting() != 0 {
                continue;
            }

            if desc.class() != MISCELLANEOUS_CLASS || desc.subclass() != USB3V_SUBCLASS {
                return None;
            }

            let iface_kind = match desc.protocol() {
                Self::EVENT_IFACE_PROTOCOL => ReceiveIfaceKind::Event,
                Self::STREAM_IFACE_PROTOCOL => ReceiveIfaceKind::Stream,
                _ => return None,
            };

            if desc.num_endpoints() != 1 {
                return None;
            }
            let ep = desc.endpoints().next().unwrap();
            if ep.transfer_type() != EndpointType::Bulk || ep.direction() != Direction::In {
                return None;
            }

            let iface_info = ReceiveIfaceInfo {
                iface_number,
                bulk_in_ep: ep.address(),
            };

            return Some((iface_info, iface_kind));
        }

        None
    }
}

#[derive(Debug, PartialEq)]
enum ReceiveIfaceKind {
    Stream,
    Event,
}
