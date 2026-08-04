#![no_std]
#![no_main]

use core::{panic::PanicInfo, ptr::addr_of_mut};

use ch32v203g6u6_embassy_hal::{
    gpio::{DRV_GPIOB_RUNTIME_RESOURCES, GPIOB},
    i2c::{DRV_I2C1_RUNTIME_RESOURCES, I2C1},
    rcc::{DRV_RCC_RUNTIME_RESOURCES, RCC},
    usb::{DRV_USBD_RUNTIME_RESOURCES, USBD, USBDUsbDriver},
    wch,
};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::Timer;
use embassy_usb::{
    Builder, Config, UsbDevice,
    class::cdc_acm::{CdcAcmClass, State as CdcState},
};
use optibridge_protocol::{
    CMD_I2C_READ, CMD_I2C_WRITE, MAX_FRAME, Parser, Request, STATUS_BAD_COMMAND, STATUS_BAD_LENGTH,
    STATUS_OK, encode_response, validate_i2c_request,
};

static mut CONFIG_DESCRIPTOR: [u8; 256] = [0; 256];
static mut BOS_DESCRIPTOR: [u8; 256] = [0; 256];
static mut MSOS_DESCRIPTOR: [u8; 256] = [0; 256];
static mut CONTROL_BUFFER: [u8; 128] = [0; 128];
static mut CDC_STATE: CdcState<'static> = CdcState::new();

const DTR_POLL_MS: u64 = 50;
const GPIOB_CFGLR: *mut u32 = 0x4001_0c00 as *mut u32;
const GPIOB_BSHR: *mut u32 = 0x4001_0c10 as *mut u32;
const PB6_MODE_SHIFT: u32 = 24;
const PB7_MODE_SHIFT: u32 = 28;
const GPIO_ALT_OPEN_DRAIN_50MHZ: u32 = 0xF;

fn configure_i2c_pins() {
    unsafe {
        GPIOB_BSHR.write_volatile((1 << 6) | (1 << 7));
        let current = GPIOB_CFGLR.read_volatile();
        let mask = (0xF << PB6_MODE_SHIFT) | (0xF << PB7_MODE_SHIFT);
        let value = (GPIO_ALT_OPEN_DRAIN_50MHZ << PB6_MODE_SHIFT)
            | (GPIO_ALT_OPEN_DRAIN_50MHZ << PB7_MODE_SHIFT);
        GPIOB_CFGLR.write_volatile((current & !mask) | value);
    }
}

fn reset_mcu() -> ! {
    const PFIC_SCTLR: *mut u32 = 0xe000_ed10 as *mut u32;
    const SYSRESET: u32 = 1 << 31;

    unsafe {
        PFIC_SCTLR.write_volatile(SYSRESET);
    }

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, USBDUsbDriver>) -> ! {
    device.run().await
}

#[embassy_executor::main(entry = "riscv_rt::entry")]
async fn main(spawner: Spawner) -> ! {
    let rcc = RCC::new(DRV_RCC_RUNTIME_RESOURCES).unwrap();
    rcc.configure_usb_fsdev_clock_48mhz().unwrap();
    wch::init_embassy_time_runtime().unwrap();

    let gpiob = GPIOB::new(DRV_GPIOB_RUNTIME_RESOURCES).unwrap();
    gpiob.enable_clock().unwrap();
    gpiob.release_reset().unwrap();
    configure_i2c_pins();

    let i2c = I2C1::new(DRV_I2C1_RUNTIME_RESOURCES).unwrap();
    i2c.enable_clock().unwrap();
    i2c.release_reset().unwrap();
    i2c.apply_init_master_100khz().unwrap();

    let usbd = USBD::new(DRV_USBD_RUNTIME_RESOURCES).unwrap();
    let driver = usbd.embassy_usb_driver();
    let mut config = Config::new(0xcafe, 0x4004);
    config.manufacturer = Some("OptiBridge");
    config.product = Some("I2C bridge");
    config.serial_number = Some("0001");
    config.max_power = 100;

    let config_descriptor = unsafe { &mut *addr_of_mut!(CONFIG_DESCRIPTOR) };
    let bos_descriptor = unsafe { &mut *addr_of_mut!(BOS_DESCRIPTOR) };
    let msos_descriptor = unsafe { &mut *addr_of_mut!(MSOS_DESCRIPTOR) };
    let control_buffer = unsafe { &mut *addr_of_mut!(CONTROL_BUFFER) };
    let cdc_state = unsafe { &mut *addr_of_mut!(CDC_STATE) };
    let mut builder = Builder::new(
        driver,
        config,
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buffer,
    );
    let mut cdc = CdcAcmClass::new(&mut builder, cdc_state, 64);
    let usb = builder.build();
    spawner.spawn(usb_task(usb)).unwrap();

    let mut parser = Parser::new();
    let mut input = [0; MAX_FRAME];
    let mut output = [0; MAX_FRAME];
    let mut i2c_read = [0; 16];

    loop {
        cdc.wait_connection().await;
        if cdc.write_packet(b"READY\r\n").await.is_err() {
            reset_mcu();
        }
        while !cdc.dtr() {
            Timer::after_millis(DTR_POLL_MS).await;
        }
        parser.reset();
        while cdc.dtr() {
            let received = match select(
                cdc.read_packet(&mut input),
                Timer::after_millis(DTR_POLL_MS),
            )
            .await
            {
                Either::First(Ok(length)) => length,
                Either::First(Err(_)) => reset_mcu(),
                Either::Second(()) => continue,
            };
            for byte in input[..received].iter().copied() {
                match parser.push(byte) {
                    Ok(Some(request)) => {
                        let length =
                            handle_request(&i2c, &cdc, request, &mut i2c_read, &mut output).await;
                        if cdc.write_packet(&output[..length]).await.is_err() {
                            reset_mcu();
                        }
                    }
                    Ok(None) => {}
                    Err(_) => parser.reset(),
                }
            }
        }

        reset_mcu();
    }
}

async fn reset_on_dtr_drop(cdc: &CdcAcmClass<'static, USBDUsbDriver>) {
    loop {
        Timer::after_millis(DTR_POLL_MS).await;
        if !cdc.dtr() {
            reset_mcu();
        }
    }
}

async fn handle_request(
    i2c: &I2C1,
    cdc: &CdcAcmClass<'static, USBDUsbDriver>,
    request: Request,
    read_buffer: &mut [u8; 16],
    output: &mut [u8; MAX_FRAME],
) -> usize {
    if let Err(status) = validate_i2c_request(&request) {
        return encode_response(status, request.sequence, 0, &[], output).unwrap_or(5);
    }
    let length = request.payload_len as usize;
    let address = request.payload[0];
    match request.command {
        CMD_I2C_WRITE => {
            let result = match select(
                i2c.write_async_7bit(address, &request.payload[1..length]),
                reset_on_dtr_drop(cdc),
            )
            .await
            {
                Either::First(result) => result,
                Either::Second(()) => reset_mcu(),
            };
            encode_response(
                if result.is_ok() {
                    STATUS_OK
                } else {
                    STATUS_BAD_COMMAND
                },
                request.sequence,
                0,
                &[],
                output,
            )
            .unwrap_or(5)
        }
        CMD_I2C_READ if length == 2 && request.payload[1] as usize <= read_buffer.len() => {
            let count = request.payload[1] as usize;
            let result = match select(
                i2c.read_async_7bit(address, &mut read_buffer[..count]),
                reset_on_dtr_drop(cdc),
            )
            .await
            {
                Either::First(result) => result,
                Either::Second(()) => reset_mcu(),
            };
            let status = if result.is_ok() {
                STATUS_OK
            } else {
                STATUS_BAD_COMMAND
            };
            let payload = if result.is_ok() {
                &read_buffer[..count]
            } else {
                &[]
            };
            encode_response(status, request.sequence, 0, payload, output).unwrap_or(5)
        }
        CMD_I2C_READ => {
            encode_response(STATUS_BAD_LENGTH, request.sequence, 0, &[], output).unwrap_or(5)
        }
        _ => unreachable!(),
    }
}
