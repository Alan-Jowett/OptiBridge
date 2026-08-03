#![no_std]
#![no_main]

use core::panic::PanicInfo;

use ch32v203g6u6_embassy_hal::i2c::{DRV_I2C1_SLAVE_RUNTIME_RESOURCES, I2C1Slave};
use embassy_executor::Spawner;
use optibridge_protocol::{MAX_FRAME, Parser, dispatch};

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[embassy_executor::main(entry = "riscv_rt::entry")]
async fn main(_spawner: Spawner) -> ! {
    let i2c = I2C1Slave::new(DRV_I2C1_SLAVE_RUNTIME_RESOURCES).unwrap();
    i2c.enable_clock().unwrap();
    i2c.release_reset().unwrap();
    i2c.init_slave().unwrap();
    i2c.set_own_address_7bit(0x42).unwrap();

    let mut parser = Parser::new();
    let mut request = [0; MAX_FRAME];
    let mut response = [0; MAX_FRAME];

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
            let length = dispatch(&request, &mut response);
            let _ = i2c.write_packet_async(&response[..length]).await;
        }
    }
}
