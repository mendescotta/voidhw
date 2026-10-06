//! Detect the hardware of a Void Linux machine and decide which driver, firmware and
//! service packages it needs. Detection only reads sysfs (rooted at a configurable
//! directory so it can be tested against snapshots of real machines); changes happen
//! only in [`apply`].

pub mod apply;
pub mod nvidia;
pub mod plan;
pub mod profile;
pub mod snapshot;
pub mod sysfs;
