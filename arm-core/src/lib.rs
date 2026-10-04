//! arm-core: general-purpose control library for the Dorna 1 arm.
//!
//! The binary (`main.rs`) and tests build on these modules. Layering:
//! `model` (domain types + safety) -> `g2core` (wire codec) -> `transport`
//! (byte I/O behind a trait) -> `controller` (state machine). Each layer is
//! unit-testable without hardware; the `Transport` trait is the seam that lets
//! the controller and codec run against canned responses.

pub mod controller;
pub mod g2core;
pub mod model;
pub mod protocol;
pub mod transport;
