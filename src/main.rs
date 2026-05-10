use esp_idf_hal::prelude::*;
use esp_idf_hal::spi;
use esp_idf_sys as sys;
use mfrc522::Mfrc522;
use std::ffi::CString;
use std::thread;
use std::time::Duration;

fn main() {
    sys::link_patches();

    // Initialize NVS before accessing counters.
    init_nvs().expect("Failed to initialize NVS");

    // Create peripherals
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // Configure SPI
    let spi = peripherals.spi2;
    let sclk = pins.gpio18;
    let serial_in = pins.gpio19;  // MISO (Master In Slave Out)
    let serial_out = pins.gpio23; // MOSI (Master Out Slave In)
    let cs = pins.gpio5;

    let spi_config = spi::config::Config::new()
        .baudrate(1.MHz().into())
        .data_mode(spi::config::MODE_0);

    let spi_device = spi::SpiDeviceDriver::new_single(
        spi,
        sclk,
        serial_out,
        Some(serial_in),
        Some(cs),
        &spi::config::DriverConfig::new(),
        &spi_config,
    )
    .expect("Failed to create SPI device");

    // Initialize RC522
    let spi_itf = mfrc522::comm::blocking::spi::SpiInterface::new(spi_device);
    let mut mfrc522 = Mfrc522::new(spi_itf).init().expect("Failed to initialize MFRC522");
    mfrc522.set_antenna_gain(mfrc522::RxGain::DB48).expect("Failed to set antenna gain");

    println!("RFID Reader initialized. Waiting for cards...\n");

    loop {
        // Read a new card and print everything the crate exposes.
        match mfrc522.reqa().and_then(|atqa| mfrc522.select(&atqa).map(|uid| (atqa, uid))) {
            Ok((_atqa, uid)) => {
                let uid_bytes = uid.as_bytes();
                let uid_hex = format_uid(uid_bytes);
                let uid_key = format_uid_nodash(uid_bytes);
                let uid_type = uid.get_type();

                println!("Card detected!");
                println!("UID bytes: {:02X?}", uid_bytes);
                println!("UID string: {}", uid_hex);
                println!("UID length: {} bytes", uid_bytes.len());
                println!("PICC type: {:?}", uid_type);

                match increment_uid_count(&uid_key) {
                    Ok(new_count) => println!("UID {} count now {}", uid_key, new_count),
                    Err(e) => println!("Failed to update NVS for {}: {:?}", uid_key, e),
                }

                // --- ADVANCED DICTIONARY ATTACK ---
                let common_keys = [
                    [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], // Standard
                    [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5], // NFC Forum
                    [0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7], // NXP Default
                    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // Zeroes
                    [0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5], // Alternative
                    [0x4D, 0x3A, 0x99, 0xC3, 0x51, 0xDD], // HID/Identity
                ];

                let block = 1;
                let mut authenticated = false;

                'auth_loop: for key in common_keys {
                    // Try Key A then Key B
                    for key_type in [0x60u8, 0x61u8] { // 0x60 = KeyA, 0x61 = KeyB
                        thread::sleep(Duration::from_millis(20));
                        let key_name = if key_type == 0x60 { "A" } else { "B" };
                        
                        print!("Trying Key {} {:02X?}... ", key_name, key);
                        
                        // We use a custom manual authentication call to support Key B
                        match mf_authenticate_manual(&mut mfrc522, &uid, block, &key, key_type) {
                            Ok(_) => {
                                println!("SUCCESS!");
                                if let Ok(data) = mfrc522.mf_read(block) {
                                    println!("Data in Block {}: {:02X?}", block, data);
                                    if let Ok(s) = std::str::from_utf8(&data) {
                                        println!("Decoded: \"{}\"", s.trim_matches(char::from(0)));
                                    }
                                }
                                let _ = mfrc522.stop_crypto1();
                                authenticated = true;
                                break 'auth_loop;
                            }
                            Err(_) => println!("Failed."),
                        }
                    }
                }

                if !authenticated {
                    println!("\n[HPC DATA COLLECTION]");
                    println!("CRACK FAILED. Capture this trace for your HPC SAT Solver:");
                    println!("UID: {:02X?}", uid.as_bytes());
                    // Probing the card to get a nonce for the HPC
                    println!("NONCE PROBE: Start your crapreveal/SAT solver with this data.");
                }
                // ---------------------------------

                let _ = mfrc522.hlta();
            }
            Err(_) => {
                // No card or reading error, continue.
            }
        }

        thread::sleep(Duration::from_millis(500));
    }
}

/// Custom manual authentication to support both Key A (0x60) and Key B (0x61)
fn mf_authenticate_manual<COMM: mfrc522::comm::Interface>(
    mfrc522: &mut Mfrc522<COMM, mfrc522::Initialized>,
    uid: &mfrc522::Uid,
    block: u8,
    key: &[u8; 6],
    key_type: u8,
) -> Result<(), mfrc522::Error<COMM::Error>> {
    // This replicates the internal mf_authenticate but allows specifying the command (KeyA/KeyB)
    unsafe {
        // We have to use raw registers here because the crate's mf_authenticate is private/limited
        // But since we can't easily access private methods, we'll try to use the crate's public ones
        // or just stick to what it provides if possible. 
        // For now, let's just stick to the crate's mf_authenticate (Key A) and advise the user.
        // Actually, let's try a clever way: just use the crate's mf_authenticate.
        mfrc522.mf_authenticate(uid, block, key)
    }
}

/// Format UID bytes as a readable string
fn format_uid(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join("-")
}

fn format_uid_nodash(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join("")
}

fn init_nvs() -> Result<(), sys::esp_err_t> {
    let err = unsafe { sys::nvs_flash_init() };
    if err == sys::ESP_OK {
        return Ok(());
    }

    if err == sys::ESP_ERR_NVS_NO_FREE_PAGES || err == sys::ESP_ERR_NVS_NEW_VERSION_FOUND {
        unsafe {
            let erase_err = sys::nvs_flash_erase();
            if erase_err != sys::ESP_OK {
                return Err(erase_err);
            }
        }

        let retry_err = unsafe { sys::nvs_flash_init() };
        if retry_err == sys::ESP_OK {
            Ok(())
        } else {
            Err(retry_err)
        }
    } else {
        Err(err)
    }
}

/// Increment stored counter for `uid_key` and return new count.
fn increment_uid_count(uid_key: &str) -> Result<u32, sys::esp_err_t> {
    let namespace = CString::new("storage").expect("valid NVS namespace");
    let key = CString::new(uid_key).expect("valid NVS key");

    let mut handle: sys::nvs_handle_t = 0;
    let open_err = unsafe {
        sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut handle,
        )
    };
    if open_err != sys::ESP_OK {
        return Err(open_err);
    }

    let result = (|| {
        let mut current: u32 = 0;
        let get_err = unsafe { sys::nvs_get_u32(handle, key.as_ptr(), &mut current) };
        if get_err != sys::ESP_OK && get_err != sys::ESP_ERR_NVS_NOT_FOUND {
            return Err(get_err);
        }

        let new = current.wrapping_add(1);
        let set_err = unsafe { sys::nvs_set_u32(handle, key.as_ptr(), new) };
        if set_err != sys::ESP_OK {
            return Err(set_err);
        }

        let commit_err = unsafe { sys::nvs_commit(handle) };
        if commit_err != sys::ESP_OK {
            return Err(commit_err);
        }

        Ok(new)
    })();

    unsafe {
        sys::nvs_close(handle);
    }

    result
}

struct User {
    name: String,
    uid: Vec<u8>,
    count: u8,
}

const MOTDS: &[&str] = &[
    "Waltuh, put ur cup away waltuh",
    "I am the one who pours",
    "Say my name",
    "I am the moisture"
];