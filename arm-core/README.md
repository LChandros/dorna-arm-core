# arm-core

A general-purpose control daemon for the Dorna 1 robotic arm. The arm is exposed as a service with a stable network API; plating, vision, and UI are all clients, not special cases. Written in Rust for a dependable long-running service.

## Why a daemon, not a library

The arm should be a thing on the network that any project can drive, the same way Keith is a capability rather than a script. One process owns the controller and serializes access to it (you cannot have two programs fighting over `/dev/ttyACM0`), enforces safety invariants in one place, and presents a clean API. Clients in any language call that API.

## Hardware it talks to

- Controller: Synthetos g2core on an Arduino Due, `/dev/ttyACM0`, JSON-over-serial at 115200. Firmware 1.21 / build 101.03. The MCU does the hard real-time work (jerk-limited multi-axis step generation, homing). This daemon does coordination, not timing.
- Host: Raspberry Pi 3B (`ultra-arm`), aarch64. Build natively on the Pi (`cargo build --release`); cross-compiling the serial stack's C deps from x86 is not worth the fragility.

## Architecture (target)

```
clients (plating recipe runner, vision node, UI)  any language
        |  network API (stable contract)
   +----v---------------------------------------+
   | api        line-delimited JSON over TCP     |
   | controller state machine: conn, homed,      |
   |            position, faults, command queue  |
   | g2core     typed JSON codec + footer        |
   |            flow control + status parsing    |
   | transport  serial port to /dev/ttyACM0      |
   +---------------------------------------------+
```

One thread owns the serial port; the API layer talks to it over channels. No async runtime, to keep compiles and memory sane on the Pi 3B and the control loop easy to reason about.

## Status

Milestone 1 (current): `src/main.rs` is a read-only smoke test that queries firmware/status over serial and prints the responses. Commands no motion. Proves the Rust to g2core path on real hardware.

Next: factor into `transport` / `g2core` / `controller` / `api` modules; add the motion command path (home, move-joint, move-cartesian, get-state, set-IO, run-sequence) with joint-limit guards; then the network API. The plating recipe runner moves to its own client repo.

## Build and run (on the Pi)

```
cargo build --release      # compile only; does not touch the serial port
./target/release/arm-core  # runs the read-only smoke test
```
