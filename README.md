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

The hardware-facing code uses
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

## Project Status

OptiBridge is under active development. This README describes the intended
architecture at a high level; protocol and runtime specifications will be added
as the design evolves.

## License

See [LICENSE](LICENSE).
