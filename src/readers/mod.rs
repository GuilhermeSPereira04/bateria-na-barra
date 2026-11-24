//! Battery readers, one per device. Each `read_once` is blocking and is meant
//! to run on a background thread.
pub mod dualsense;
pub mod adb;
pub mod zaopin;
