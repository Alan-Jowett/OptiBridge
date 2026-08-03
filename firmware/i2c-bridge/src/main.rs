#![no_std]
#![no_main]

use core::{panic::PanicInfo, ptr::addr_of_mut};

use ch32v203g6u6_embassy_hal::{
    i2c::{DRV_I2C1_RUNTIME_RESOURCES, I2C1},
    rcc::{DRV_RCC_RUNTIME_RESOURCES, RCC},
    usb::{DRV_USBD_RUNTIME_RESOURCES, USBD, USBDUsbDriver},
    wch,
};
use embassy_executor::Spawner;
use embassy_usb::{
    Builder, Config, UsbDevice,
    class::cdc_acm::{CdcAcmClass, State as CdcState},
};
use optibridge_protocol::{
    CMD_I2C_READ, CMD_I2C_WRITE, MAX_FRAME, Parser, Request, STATUS_BAD_COMMAND, STATUS_BAD_LENGTH,
    STATUS_OK, encode_response,
};

static mut CONFIG_DESCRIPTOR: [u8; 256] = [0; 256];
static mut BOS_DESCRIPTOR: [u8; 256] = [0; 256];
static mut MSOS_DESCRIPTOR: [u8; 256] = [0; 256];
static mut CONTROL_BUFFER: [u8; 128] = [0; 128];
static mut CDC_STATE: CdcState<'static> = CdcState::new();

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
        while !cdc.dtr() {
            core::future::pending::<()>().await;
        }
        while cdc.dtr() {
            let received = match cdc.read_packet(&mut input).await {
                Ok(length) => length,
                Err(_) => break,
            };
            parser.reset();
            let mut request = None;
            for byte in input[..received].iter().copied() {
                match parser.push(byte) {
                    Ok(Some(value)) => request = Some(value),
                    Ok(None) => {}
                    Err(_) => {
                        parser.reset();
                        break;
                    }
                }
            }
            let length = handle_request(&i2c, request, &mut i2c_read, &mut output);
            if cdc.write_packet(&output[..length]).await.is_err() {
                break;
            }
        }
    }
}

fn handle_request(
    i2c: &I2C1,
    request: Option<Request>,
    read_buffer: &mut [u8; 16],
    output: &mut [u8; MAX_FRAME],
) -> usize {
    let Some(request) = request else {
        return encode_response(STATUS_BAD_LENGTH, 0, 0, &[], output).unwrap_or(5);
    };
    if request.flags != 0 {
        return encode_response(STATUS_BAD_COMMAND, request.sequence, 0, &[], output).unwrap_or(5);
    }
    let length = request.payload_len as usize;
    if length == 0 {
        return encode_response(STATUS_BAD_LENGTH, request.sequence, 0, &[], output).unwrap_or(5);
    }
    let address = request.payload[0];
    match request.command {
        CMD_I2C_WRITE => {
            let result = i2c.blocking_write_7bit(address, &request.payload[1..length]);
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
            let status = if i2c
                .blocking_read_7bit(address, &mut read_buffer[..count])
                .is_ok()
            {
                STATUS_OK
            } else {
                STATUS_BAD_COMMAND
            };
            encode_response(status, request.sequence, 0, &read_buffer[..count], output).unwrap_or(5)
        }
        CMD_I2C_READ => {
            encode_response(STATUS_BAD_LENGTH, request.sequence, 0, &[], output).unwrap_or(5)
        }
        _ => encode_response(STATUS_BAD_COMMAND, request.sequence, 0, &[], output).unwrap_or(5),
    }
}
