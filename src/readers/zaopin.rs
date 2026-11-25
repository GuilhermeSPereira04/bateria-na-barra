//! Zaopin mouse (Compx-based): 2.4 GHz receiver 3554:F527, or the mouse
//! itself on its USB cable, 3554:F526 — same protocol on both.
//!
//! Battery query taken from the official driver (HIDUsb.dll,
//! `UsbServer_ReadBatteryLevel` / `CS_UsbFinder_GetBatteryLevel` in mouse
//! mode): output report 0x08 on the vendor collection 0xFF02,
//! `08 04 00*14 cs`, with `cs = 0x55 - sum(bytes 0..16)`. Reply is input
//! report 0x08: byte 1 echoes 0x04, byte 6 = percent, byte 7 > 0 = charging,
//! bytes 8..9 = millivolts (big endian), byte 16 = checksum.
//! This module sends that exact request and nothing else.
use crate::model::{Charge, ReadOutcome, Reading};

pub const VID: u16 = 0x3554;
/// Receiver (2.4 GHz) and wired mouse; the official software lists both.
pub const PIDS: [u16; 2] = [0xF527, 0xF526];
const USAGE_PAGE: u16 = 0xFF02;
const REPORT_ID: u8 = 0x08;
const CMD_BATTERY: u8 = 0x04;

pub fn is_zaopin(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

fn checksum(bytes: &[u8]) -> u8 {
    0x55u8.wrapping_sub(bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)))
}

pub fn battery_request() -> [u8; 17] {
    let mut req = [0u8; 17];
    req[0] = REPORT_ID;
    req[1] = CMD_BATTERY;
    req[16] = checksum(&req[..16]);
    req
}

pub fn parse_reply(buf: &[u8]) -> Option<Reading> {
    if buf.len() < 17 || buf[0] != REPORT_ID || buf[1] != CMD_BATTERY || buf[16] != checksum(&buf[..16]) {
        return None;
    }
    let percent = buf[6].min(100);
    let charge = match (buf[7] > 0, percent) {
        (true, 100) => Charge::Full,
        (true, _) => Charge::Charging,
        (false, _) => Charge::Discharging,
    };
    Some(Reading { percent, charge })
}

/// Sends the battery query (up to 3 tries, 100 ms each, like the driver).
/// Neither receiver nor cable → `Offline`; no valid reply (e.g. mouse asleep) → `Failed`.
pub fn read_once(api: &mut hidapi::HidApi) -> ReadOutcome {
    // Enumerate only the Zaopin devices instead of querying every HID device.
    api.reset_devices().ok();
    for pid in PIDS {
        if api.add_devices(VID, pid).is_err() {
            return ReadOutcome::Failed;
        }
    }
    let Some(info) = api
        .device_list()
        .find(|d| is_zaopin(d.vendor_id(), d.product_id()) && d.usage_page() == USAGE_PAGE)
    else {
        return ReadOutcome::Offline;
    };
    let Ok(dev) = info.open_device(api) else {
        return ReadOutcome::Failed;
    };
    let req = battery_request();
    let mut buf = [0u8; 64];
    for _ in 0..3 {
        if dev.write(&req).is_err() {
            return ReadOutcome::Failed;
        }
        for _ in 0..5 {
            match dev.read_timeout(&mut buf, 100) {
                Ok(n) if n > 0 => {
                    if let Some(reading) = parse_reply(&buf[..n]) {
                        return ReadOutcome::Ok(reading);
                    }
                }
                Ok(_) => break, // timed out: resend
                Err(_) => return ReadOutcome::Failed,
            }
        }
    }
    ReadOutcome::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from the user's receiver on 2026-11-24.
    const REAL_REPLY: [u8; 17] =
        [0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x19, 0x00, 0x0E, 0x3D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xE3];

    fn with_checksum(mut r: [u8; 17]) -> [u8; 17] {
        r[16] = checksum(&r[..16]);
        r
    }

    #[test]
    fn request_matches_official_driver() {
        let mut expected = [0u8; 17];
        expected[0] = 0x08;
        expected[1] = 0x04;
        expected[16] = 0x49;
        assert_eq!(battery_request(), expected);
    }

    #[test]
    fn parses_real_reply() {
        assert_eq!(parse_reply(&REAL_REPLY), Some(Reading { percent: 25, charge: Charge::Discharging }));
    }

    #[test]
    fn second_real_reply_has_valid_checksum() {
        let r = [0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x19, 0x00, 0x0E, 0x39, 0, 0, 0, 0, 0, 0, 0xE7];
        assert_eq!(parse_reply(&r), Some(Reading { percent: 25, charge: Charge::Discharging }));
    }

    #[test]
    fn parses_real_charging_reply_over_cable() {
        // Captured with the mouse on its USB cable (PID F526).
        let r = [0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x28, 0x01, 0x0E, 0xBF, 0, 0, 0, 0, 0, 0, 0x51];
        assert_eq!(parse_reply(&r), Some(Reading { percent: 40, charge: Charge::Charging }));
    }

    #[test]
    fn wired_and_wireless_are_supported() {
        assert!(is_zaopin(0x3554, 0xF527)); // 2.4 GHz receiver
        assert!(is_zaopin(0x3554, 0xF526)); // mouse on its USB cable
        assert!(!is_zaopin(0x3554, 0x1234));
        assert!(!is_zaopin(0x054C, 0xF527));
    }

    #[test]
    fn charging_flag() {
        let mut r = REAL_REPLY;
        r[7] = 1;
        assert_eq!(parse_reply(&with_checksum(r)), Some(Reading { percent: 25, charge: Charge::Charging }));
    }

    #[test]
    fn full_when_charging_at_100() {
        let mut r = REAL_REPLY;
        r[6] = 100;
        r[7] = 1;
        assert_eq!(parse_reply(&with_checksum(r)), Some(Reading { percent: 100, charge: Charge::Full }));
    }

    #[test]
    fn clamps_over_100() {
        let mut r = REAL_REPLY;
        r[6] = 120;
        assert_eq!(parse_reply(&with_checksum(r)).map(|x| x.percent), Some(100));
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut r = REAL_REPLY;
        r[16] ^= 1;
        assert_eq!(parse_reply(&r), None);
    }

    #[test]
    fn rejects_other_command_or_report() {
        let mut r = REAL_REPLY;
        r[1] = 0x05;
        assert_eq!(parse_reply(&with_checksum(r)), None);
        let mut r = REAL_REPLY;
        r[0] = 0x09;
        assert_eq!(parse_reply(&with_checksum(r)), None);
    }

    #[test]
    fn rejects_short() {
        assert_eq!(parse_reply(&REAL_REPLY[..10]), None);
    }
}
