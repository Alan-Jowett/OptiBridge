#![no_std]
#![no_main]

use core::panic::PanicInfo;

use ch32v203g6u6_embassy_hal::{
    gpio::{DRV_GPIOB_RUNTIME_RESOURCES, GPIOB},
    i2c::{DRV_I2C1_SLAVE_RUNTIME_RESOURCES, I2C1Slave},
    rcc::{DRV_RCC_RUNTIME_RESOURCES, RCC},
    wch,
};
use embassy_executor::Spawner;
use optibridge_protocol::{MAX_FRAME, Parser, StatusQueue, dispatch};

const GPIOB_CFGLR: *mut u32 = 0x4001_0c00 as *mut u32;
const GPIOB_BSHR: *mut u32 = 0x4001_0c10 as *mut u32;
const RCC_APB2PCENR: *mut u32 = 0x4002_1018 as *mut u32;
const AFIO_PCFR1: *mut u32 = 0x4001_0004 as *mut u32;
const PB6_MODE_SHIFT: u32 = 24;
const PB7_MODE_SHIFT: u32 = 28;
const GPIO_ALT_OPEN_DRAIN_50MHZ: u32 = 0xF;
const BPF_SIZE_PROBE: [u8; 16] = [
    0xb7, 0x00, 0x00, 0x00, 0x2a, 0x00, 0x00, 0x00, // mov64 r0, 42
    0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // exit
];

fn configure_i2c_pins() {
    unsafe {
        RCC_APB2PCENR.write_volatile(RCC_APB2PCENR.read_volatile() | 1);
        AFIO_PCFR1.write_volatile(AFIO_PCFR1.read_volatile() & !2);
        GPIOB_BSHR.write_volatile((1 << 6) | (1 << 7));
        let current = GPIOB_CFGLR.read_volatile();
        let mask = (0xF << PB6_MODE_SHIFT) | (0xF << PB7_MODE_SHIFT);
        let value = (GPIO_ALT_OPEN_DRAIN_50MHZ << PB6_MODE_SHIFT)
            | (GPIO_ALT_OPEN_DRAIN_50MHZ << PB7_MODE_SHIFT);
        GPIOB_CFGLR.write_volatile((current & !mask) | value);
    }
}

#[inline(never)]
fn run_bpf_size_probe() {
    let helpers: &[sonde_bpf::interpreter::HelperDescriptor] = &[];
    let mut context = [0; 1];
    let result = sonde_bpf::interpreter::execute_program_no_maps(
        core::hint::black_box(&BPF_SIZE_PROBE),
        core::hint::black_box(&mut context),
        core::hint::black_box(helpers),
        false,
        core::hint::black_box(2),
    );
    if !matches!(core::hint::black_box(result), Ok(42)) {
        panic!();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[embassy_executor::main(entry = "riscv_rt::entry")]
async fn main(_spawner: Spawner) -> ! {
    run_bpf_size_probe();

    let rcc = RCC::new(DRV_RCC_RUNTIME_RESOURCES).unwrap();
    rcc.configure_usb_fsdev_clock_48mhz().unwrap();
    wch::init_embassy_time_runtime().unwrap();

    let gpiob = GPIOB::new(DRV_GPIOB_RUNTIME_RESOURCES).unwrap();
    gpiob.enable_clock().unwrap();
    gpiob.release_reset().unwrap();
    configure_i2c_pins();

    let i2c = I2C1Slave::new(DRV_I2C1_SLAVE_RUNTIME_RESOURCES).unwrap();
    i2c.enable_clock().unwrap();
    i2c.release_reset().unwrap();
    i2c.init_slave().unwrap();
    i2c.set_own_address_7bit(0x42).unwrap();

    let mut parser = Parser::new();
    let mut request = [0; MAX_FRAME];
    let mut response = [0; MAX_FRAME];
    let mut status_queue = StatusQueue::ready();

    loop {
        let received = match i2c.read_packet_async(&mut request).await {
            Ok(length) => length,
            Err(_) => {
                parser.reset();
                continue;
            }
        };
        parser.reset();
        let mut parsed = None;
        for byte in request[..received].iter().copied() {
            match parser.push(byte) {
                Ok(Some(value)) => parsed = Some(value),
                Ok(None) => {}
                Err(_) => {
                    parser.reset();
                    break;
                }
            }
        }
        if let Some(request) = parsed {
            let length = dispatch(&request, &mut status_queue, &mut response);
            let _ = i2c.write_packet_async(&response[..length]).await;
        }
    }
}
