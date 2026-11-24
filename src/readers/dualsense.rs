//! DualSense over Bluetooth — passive only.
//!
//! This module never writes to the controller (no output reports, no feature
//! reports): switching it into the full `0x31` report mode would break
//! DS4Windows/emulators. If the controller is only sending the simple `0x01`
//! report, the battery is reported as unreadable.
#![allow(dead_code)]

use crate::model::{Charge, ReadOutcome, Reading};

const VID_SONY: u16 = 0x054C;
const PIDS: [u16; 2] = [0x0CE6, 0x0DF2]; // DualSense, DualSense Edge
const BT_FULL_REPORT_ID: u8 = 0x31;
const BT_FULL_REPORT_LEN: usize = 78;
/// Status byte: report id (1) + bt tag (1) + offset 52 in the common input report.
const STATUS_OFFSET: usize = 54;

pub fn parse_bt_report(buf: &[u8]) -> Option<Reading> {
    if buf.len() < BT_FULL_REPORT_LEN || buf[0] != BT_FULL_REPORT_ID {
        return None;
    }
    let status = buf[STATUS_OFFSET];
    let data = status & 0x0F;
    let percent = (data * 10 + 5).min(100);
    match status >> 4 {
        0x0 => Some(Reading { percent, charge: Charge::Discharging }),
        0x1 => Some(Reading { percent, charge: Charge::Charging }),
        0x2 => Some(Reading { percent: 100, charge: Charge::Full }),
        _ => None,
    }
}

/// Listens to a handful of input reports. Never writes anything to the device.
pub fn read_once(api: &mut hidapi::HidApi) -> ReadOutcome {
    if api.refresh_devices().is_err() {
        return ReadOutcome::Failed;
    }
    let Some(path) = api
        .device_list()
        .find(|d| d.vendor_id() == VID_SONY && PIDS.contains(&d.product_id()))
        .map(|d| d.path().to_owned())
    else {
        return ReadOutcome::Offline;
    };
    // Fails when DS4Windows hides the controller with HidHide.
    let Ok(device) = api.open_path(&path) else {
        return ReadOutcome::Failed;
    };
    let mut buf = [0u8; 128];
    for _ in 0..20 {
        match device.read_timeout(&mut buf, 100) {
            Ok(n) if n > 0 => {
                if let Some(reading) = parse_bt_report(&buf[..n]) {
                    return ReadOutcome::Ok(reading);
                }
            }
            Ok(_) => {}
            Err(_) => return ReadOutcome::Failed,
        }
    }
    ReadOutcome::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(status: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 78];
        buf[0] = 0x31;
        buf[54] = status;
        buf
    }

    #[test]
    fn parses_discharging() {
        assert_eq!(parse_bt_report(&report(0x06)), Some(Reading { percent: 65, charge: Charge::Discharging }));
    }

    #[test]
    fn parses_charging() {
        assert_eq!(parse_bt_report(&report(0x13)), Some(Reading { percent: 35, charge: Charge::Charging }));
    }

    #[test]
    fn parses_full() {
        assert_eq!(parse_bt_report(&report(0x2A)), Some(Reading { percent: 100, charge: Charge::Full }));
    }

    #[test]
    fn clamps_to_100() {
        assert_eq!(parse_bt_report(&report(0x0A)), Some(Reading { percent: 100, charge: Charge::Discharging }));
    }

    #[test]
    fn rejects_simple_report() {
        let mut buf = report(0x06);
        buf[0] = 0x01;
        assert_eq!(parse_bt_report(&buf), None);
    }

    #[test]
    fn rejects_short() {
        assert_eq!(parse_bt_report(&[0x31; 40]), None);
    }

    #[test]
    fn rejects_error_state() {
        assert_eq!(parse_bt_report(&report(0xF5)), None);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn ds_once() {
        let mut api = hidapi::HidApi::new().unwrap();
        for d in api.device_list().filter(|d| d.vendor_id() == 0x054C) {
            println!("found {:04x}:{:04x} up={:04x} u={:04x} {:?}", d.vendor_id(), d.product_id(), d.usage_page(), d.usage(), d.path());
        }
        println!("read_once = {:?}", super::read_once(&mut api));
    }
}
