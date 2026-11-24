//! Zaopin receiver (3554:F527). The battery protocol is not known yet, so this
//! only checks whether the receiver is plugged in. It never opens or writes to it.
use crate::model::ReadOutcome;

pub const VID: u16 = 0x3554;
pub const PID: u16 = 0xF527;

/// Receiver present → `Failed` (shown as `--%` until the protocol is captured);
/// absent → `Offline`.
pub fn read_once(api: &mut hidapi::HidApi) -> ReadOutcome {
    if api.refresh_devices().is_err() {
        return ReadOutcome::Failed;
    }
    let present = api.device_list().any(|d| d.vendor_id() == VID && d.product_id() == PID);
    if present { ReadOutcome::Failed } else { ReadOutcome::Offline }
}
