#![no_std]
#![no_main]

use core::{
    cell::RefCell,
    panic::PanicInfo,
    ptr::addr_of_mut,
    sync::atomic::{AtomicBool, Ordering},
};

use ch32v203g6u6_embassy_hal::{
    flash::{DRV_FLASH_RUNTIME_RESOURCES, FLASH},
    gpio::{DRV_GPIOB_RUNTIME_RESOURCES, GPIOB},
    i2c::{
        DRV_I2C1_SLAVE_RUNTIME_RESOURCES, I2C1Slave, queue_drv_i2c1_slave_i2c_slave_isr_tx_packet,
    },
    interrupt::system_reset,
    rcc::{DRV_RCC_RUNTIME_RESOURCES, RCC},
    wch,
};
use critical_section::Mutex;
use embassy_executor::Spawner;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use optibridge_protocol::{
    BPF_FLASH_OFFSET, BPF_HEADER_SIZE, BPF_MAX_MAPS, BPF_MAX_MAP_BYTES, BpfFlash, BpfLoader,
    BpfProgramMetadata, MAX_FRAME, PacketOutcome, StatusQueue, STATUS_BAD_COMMAND,
    dispatch_with_bpf_and_executor, parse_packet,
};

const GPIOB_CFGLR: *mut u32 = 0x4001_0c00 as *mut u32;
const GPIOB_BSHR: *mut u32 = 0x4001_0c10 as *mut u32;
const RCC_APB2PCENR: *mut u32 = 0x4002_1018 as *mut u32;
const AFIO_PCFR1: *mut u32 = 0x4001_0004 as *mut u32;
const I2C1_STAR2: *const u16 = 0x4000_5418 as *const u16;
const I2C_STAR2_BUSY: u16 = 1 << 1;
const I2C_BUS_IDLE_SPINS: u32 = 48_000;
const PB6_MODE_SHIFT: u32 = 24;
const PB7_MODE_SHIFT: u32 = 28;
const GPIO_ALT_OPEN_DRAIN_50MHZ: u32 = 0xF;
const BPF_EXECUTION_FLASH_BASE: usize = 0;
const BPF_SIZE_PROBE: [u8; 16] = [
    0xb7, 0x00, 0x00, 0x00, 0x2a, 0x00, 0x00, 0x00, // mov64 r0, 42
    0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // exit
];

struct ProtocolState {
    status_queue: StatusQueue,
    loader: BpfLoader,
    response: [u8; MAX_FRAME],
}

impl ProtocolState {
    const fn new() -> Self {
        Self {
            status_queue: StatusQueue::ready(),
            loader: BpfLoader::new(),
            response: [0; MAX_FRAME],
        }
    }
}

static PROTOCOL_STATE: Mutex<RefCell<ProtocolState>> =
    Mutex::new(RefCell::new(ProtocolState::new()));
static mut I2C_RX_BUFFER: [u8; MAX_FRAME] = [0; MAX_FRAME];
#[used]
#[unsafe(no_mangle)]
static MAP_BACKING_STORE: Mutex<RefCell<[u8; BPF_MAX_MAP_BYTES]>> =
    Mutex::new(RefCell::new([0; BPF_MAX_MAP_BYTES]));
static RESET_PENDING: AtomicBool = AtomicBool::new(false);

struct FlashStorage<'a>(&'a mut FLASH);

impl BpfFlash for FlashStorage<'_> {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), ()> {
        self.0.read(offset, bytes).map_err(|_| ())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ()> {
        self.0.write(offset, bytes).map_err(|_| ())
    }

    fn erase(&mut self, from: u32, to: u32) -> Result<(), ()> {
        self.0.erase(from, to).map_err(|_| ())
    }
}

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

fn wait_for_i2c_bus_idle() -> bool {
    for _ in 0..I2C_BUS_IDLE_SPINS {
        if unsafe { I2C1_STAR2.read_volatile() } & I2C_STAR2_BUSY == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
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

fn execute_loaded_bpf(
    metadata: &BpfProgramMetadata,
    map_backing: &mut [u8; BPF_MAX_MAP_BYTES],
) -> Result<u64, u8> {
    let mut maps = [sonde_bpf::interpreter::MapRegion {
        relocated_ptr: 0,
        key_size: 0,
        value_size: 0,
        data_start: 0,
        data_end: 0,
    }; BPF_MAX_MAPS];
    for (index, map) in maps[..metadata.map_count as usize].iter_mut().enumerate() {
        let descriptor = metadata.maps[index];
        let backing = unsafe {
            map_backing
                .as_mut_ptr()
                .add(descriptor.backing_offset as usize)
        };
        let start = backing as u64;
        let end = start + descriptor.backing_len as u64;
        *map = sonde_bpf::interpreter::MapRegion {
            relocated_ptr: start,
            key_size: descriptor.key_size,
            value_size: descriptor.value_size,
            data_start: start,
            data_end: end,
        };
    }

    let program = unsafe {
        core::slice::from_raw_parts(
            (BPF_EXECUTION_FLASH_BASE + BPF_FLASH_OFFSET as usize + BPF_HEADER_SIZE)
                as *const u8,
            metadata.bytecode_len as usize,
        )
    };
    let mut context = [];
    unsafe {
        sonde_bpf::interpreter::execute_program(
            program,
            &mut context,
            &[],
            &maps[..metadata.map_count as usize],
            false,
            sonde_bpf::interpreter::UNLIMITED_BUDGET,
            &[],
        )
    }
    .map_err(|_| STATUS_BAD_COMMAND)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn on_i2c_packet(packet: &[u8], truncated: bool) {
    let mut snapshot = [0; MAX_FRAME];
    let outcome = match parse_packet(packet, truncated) {
        Some(request) => critical_section::with(|cs| {
            let mut state = PROTOCOL_STATE.borrow(cs).borrow_mut();
            let mut map_backing = MAP_BACKING_STORE.borrow(cs).borrow_mut();
            let ProtocolState {
                status_queue,
                loader,
                response,
            } = &mut *state;
            match dispatch_with_bpf_and_executor(
                &request,
                status_queue,
                loader,
                &mut map_backing,
                response,
                execute_loaded_bpf,
            ) {
                PacketOutcome::Response(length) => {
                    snapshot[..length].copy_from_slice(&state.response[..length]);
                    PacketOutcome::Response(length)
                }
                PacketOutcome::Empty => PacketOutcome::Empty,
                PacketOutcome::Reset => PacketOutcome::Reset,
            }
        }),
        None => PacketOutcome::Empty,
    };

    match outcome {
        PacketOutcome::Response(length) => {
            let _ = queue_drv_i2c1_slave_i2c_slave_isr_tx_packet(&snapshot[..length]);
        }
        PacketOutcome::Empty => {
            let _ = queue_drv_i2c1_slave_i2c_slave_isr_tx_packet(&[]);
        }
        PacketOutcome::Reset => RESET_PENDING.store(true, Ordering::Release),
    }
}

#[embassy_executor::main(entry = "riscv_rt::entry")]
async fn main(_spawner: Spawner) -> ! {
    run_bpf_size_probe();

    let rcc = RCC::new(DRV_RCC_RUNTIME_RESOURCES).unwrap();
    rcc.configure_usb_fsdev_clock_48mhz().unwrap();
    wch::init_embassy_time_runtime().unwrap();
    let mut flash = FLASH::new(DRV_FLASH_RUNTIME_RESOURCES).unwrap();
    critical_section::with(|cs| {
        MAP_BACKING_STORE.borrow(cs).borrow_mut().fill(0);
        let mut state = PROTOCOL_STATE.borrow(cs).borrow_mut();
        state
            .loader
            .validate_committed(&mut FlashStorage(&mut flash));
    });

    let gpiob = GPIOB::new(DRV_GPIOB_RUNTIME_RESOURCES).unwrap();
    gpiob.enable_clock().unwrap();
    gpiob.release_reset().unwrap();
    configure_i2c_pins();

    let i2c = I2C1Slave::new(DRV_I2C1_SLAVE_RUNTIME_RESOURCES).unwrap();
    i2c.enable_clock().unwrap();
    i2c.release_reset().unwrap();
    i2c.init_slave().unwrap();
    i2c.set_own_address_7bit(0x42).unwrap();
    i2c.enable_rx_packet_isr_dispatch(unsafe { &mut *addr_of_mut!(I2C_RX_BUFFER) }, on_i2c_packet)
        .unwrap();

    loop {
        if RESET_PENDING.load(Ordering::Acquire) {
            system_reset();
        }
        let load_pending =
            critical_section::with(|cs| PROTOCOL_STATE.borrow(cs).borrow().loader.has_pending());
        if load_pending {
            if !wait_for_i2c_bus_idle() {
                continue;
            }
        }
        critical_section::with(|cs| {
            let mut state = PROTOCOL_STATE.borrow(cs).borrow_mut();
            state.loader.execute_pending(&mut FlashStorage(&mut flash));
        });
        core::hint::spin_loop();
    }
}
