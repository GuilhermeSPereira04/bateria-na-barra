//! Lists the Zaopin receiver's HID collections and report descriptors.
//! Read-only: it never writes, sends or requests any report.
const VID: u16 = 0x3554;
const PIDS: [u16; 2] = [0xF527, 0xF526];

fn main() {
    let api = hidapi::HidApi::new().expect("hidapi init");
    let mut found = 0;
    for d in api.device_list().filter(|d| d.vendor_id() == VID && PIDS.contains(&d.product_id())) {
        found += 1;
        println!(
            "iface={} usage_page=0x{:04X} usage=0x{:04X} product={:?}\n  path={:?}",
            d.interface_number(),
            d.usage_page(),
            d.usage(),
            d.product_string(),
            d.path()
        );
        match d.open_device(&api) {
            Ok(dev) => {
                let mut desc = [0u8; 4096];
                match dev.get_report_descriptor(&mut desc) {
                    Ok(n) => {
                        let hex: Vec<String> = desc[..n].iter().map(|b| format!("{b:02X}")).collect();
                        println!("  descriptor ({n} bytes): {}", hex.join(" "));
                    }
                    Err(e) => println!("  descriptor error: {e}"),
                }
            }
            Err(e) => println!("  open error: {e}"),
        }
    }
    println!("{found} collection(s) found");
}
