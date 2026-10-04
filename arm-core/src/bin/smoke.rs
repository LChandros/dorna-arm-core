//! Read-only hardware smoke check, dogfooding the library stack
//! (SerialTransport -> Controller). Queries firmware/status over serial and
//! prints it. Commands no motion. Quick way to confirm the controller is
//! reachable:  cargo run --release --bin smoke
//!
//! Uses the stable by-id device path (the controller's ttyACM number changes
//! across resets, but the by-id symlink does not).

use anyhow::Result;

use arm_core::controller::Controller;
use arm_core::transport::SerialTransport;

const SERIAL: &str = "/dev/serial/by-id/usb-Synthetos_TinyG_v2_0084-084a-0084-7bd6-if00";
const BAUD: u32 = 115_200;

fn main() -> Result<()> {
    let mut transport = SerialTransport::open(SERIAL, BAUD)?;
    transport.clear_input().ok();
    let mut ctrl = Controller::new(transport);

    for key in ["fv", "fb", "hp", "id"] {
        println!("{key} = {}", ctrl.query(key)?);
    }
    println!("status = {:?}", ctrl.status()?);
    Ok(())
}
