//! Sends the Zaopin battery query exactly as the official driver's
//! HIDUsb.dll (CS_UsbFinder_GetBatteryLevel, mouse mode) builds it, and prints
//! the raw reply. It sends nothing else.
//!
//! Request (output report 0x08, 17 bytes, collection usage page 0xFF02):
//!   08 04 00*14 cs   with cs = 0x55 - sum(bytes 0..16)
const VID: u16 = 0x3554;
const PID: u16 = 0xF527;

fn checksum(buf: &[u8]) -> u8 {
    0x55u8.wrapping_sub(buf.iter().fold(0u8, |a, b| a.wrapping_add(*b)))
}

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi init");
    let Some(info) = api
        .device_list()
        .find(|d| d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == 0xFF02)
    else {
        println!("receiver collection 0xFF02 not found");
        return;
    };
    let dev = info.open_device(&api).expect("open 0xFF02");
    let mut req = [0u8; 17];
    req[0] = 0x08;
    req[1] = 0x04;
    req[16] = checksum(&req[..16]);
    println!("request: {:02X?}", req);
    for attempt in 1..=3 {
        dev.write(&req).expect("write");
        let mut buf = [0u8; 64];
        for _ in 0..5 {
            match dev.read_timeout(&mut buf, 100) {
                Ok(0) => {}
                Ok(n) => {
                    let r = &buf[..n];
                    let ok = n >= 17 && r[0] == 0x08 && r[1] == 0x04 && r[16] == checksum(&r[..16]);
                    println!("attempt {attempt}: reply {:02X?} valid={ok}", r);
                    if ok {
                        println!(
                            "byte6={} byte7={} voltage(be 8..9)={} mV",
                            r[6],
                            r[7],
                            u16::from_be_bytes([r[8], r[9]])
                        );
                        return;
                    }
                }
                Err(e) => println!("read error: {e}"),
            }
        }
    }
    println!("no valid reply");
}
