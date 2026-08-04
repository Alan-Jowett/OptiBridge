# OptiBridge

OptiBridge is a programmable I2C-to-optical bridge. It provides a small,
event-driven runtime that can execute a loaded BPF program and connect that
program to the device's optical input and output channels.

The device is intended to make optical sensing and control programmable without
requiring application-specific firmware for every use case.

## Hardware

The hardware provides optical input channels based on photodiodes and optical
output channels driven by IR emitters. An MCU manages the channels and exposes
the device exclusively through an I2C interface.

The hardware-facing code is intended to use
[HardwareAbstractionIR](https://github.com/Alan-Jowett/HardwareAbstractionIR/)
to keep the runtime portable across supported hardware implementations.

## BPF Runtime

OptiBridge accepts a small BPF program of up to 1,024 BPF instructions. Programs
are loaded over multiple I2C write transactions, then started by the host.

BPF programs can:

- Read and control optical input and output channels through BPF helper calls.
- Read and update array-style BPF maps.
- Emit diagnostic status messages to a device-managed circular buffer.

The initial runtime intentionally supports only array-style maps. Hash tables,
ring buffers, and other advanced BPF map types are not supported.

## I2C Interface

The I2C interface exposes a small command set:

1. **Reset** - Reboot the MCU and clear runtime RAM state.
2. **Load BPF program** - Transfer a BPF program over one or more write
   transactions.
3. **Start BPF program** - Begin execution of the loaded program.
4. **Read BPF map state** - Read values from array-style maps.
5. **Write BPF map state** - Update values in array-style maps.
6. **Read status messages** - Retrieve diagnostic messages from the circular
   status buffer.

The detailed command encoding, register layout, verifier rules, helper
definitions, and execution semantics will be defined in the project
specifications.

## Firmware

The firmware provides the I2C command interface, manages program loading and
execution, verifies and runs BPF programs, maintains array-style maps, and
bridges BPF helper calls to the optical hardware.

It also owns reset handling and the circular status buffer used for diagnostics.
Implementation details and resource limits beyond the 1,024-instruction program
limit will be documented separately.

### Initial firmware workspace

The repository contains two firmware packages and a shared, allocation-free
protocol crate. Generate the external CH32V203G6U6 HAL before building firmware:

```powershell
cargo xtask generate-hal
cargo test -p optibridge-protocol
cargo build --release -p optibridge-firmware --target riscv32imc-unknown-none-elf --features firmware
cargo build --release -p i2c-bridge-firmware --target riscv32imc-unknown-none-elf --features firmware
cargo xtask size
```

The generated HAL is a path dependency of both firmware packages, so HAL
generation must run before Cargo resolves either firmware package.

HAL source is generated under `.generated/` from the pinned upstream
`HardwareAbstractionIR` repository and is intentionally not vendored.

The initial OptiBridge protocol is a compact binary frame:

```text
request/response: magic, command-or-status, payload-length, sequence, flags, payload
```

The six action commands are Reset (`0x01`), Load BPF (`0x02`), Start BPF
(`0x03`), Read BPF map (`0x04`), Write BPF map (`0x05`), and Read Status
(`0x06`). Payloads are bounded to 16 bytes; reserved flags must be zero.

Only Read Status is currently functional: it returns the non-destructive
startup status payload `ready`. The other five actions return
`STATUS_NOT_IMPLEMENTED`; they do not yet reset the MCU or implement BPF, map,
or optical behavior.

The USB bridge uses the same frame format:

| Command | Payload | Response |
|---|---|---|
| `0x10` I2C write | `[address, bytes...]` | status-only response |
| `0x11` I2C read | `[address, length]` | status plus read bytes |

I2C addresses are seven-bit values. Read lengths are limited to 16 bytes.
Malformed frames, unsupported commands, and failed I2C operations return a
nonzero status response. `cargo xtask size` fails when either release image's
flash sections exceed 32 KiB.

### BPF interpreter footprint

OptiBridge consumes the upstream
[`sonde-bpf`](https://github.com/Alan-Jowett/sonde/tree/main/crates/sonde-bpf)
crate at pinned revision `cacc8edfbd59c09d9425aa6b93939c7bf85ff8d5`, with
default features disabled. The firmware runs a bounded two-instruction,
allocation-free startup probe so release linking retains the interpreter for
size measurement.

| Image configuration | Flash |
| --- | ---: |
| I2C slave baseline | 8,078 bytes |
| With retained `sonde-bpf` interpreter | 21,756 bytes |
| Interpreter increment | 13,678 bytes |
| Remaining within 32 KiB | 11,012 bytes |

This probe does not implement BPF loading, helpers, maps, or optical runtime
execution.

## Project Status

OptiBridge is under active development. This README describes the intended
architecture at a high level; protocol and runtime specifications will be added
as the design evolves.

## License

See [LICENSE](LICENSE).
