#![no_std]

pub use optibridge_protocol::{
    BpfFlash, BpfLoader, PacketOutcome, StatusQueue, dispatch, dispatch_packet,
    dispatch_packet_with_bpf, dispatch_with_bpf,
};
