// Prelude removed in 0.44+
use esp_idf_hal::spi;
use esp_idf_hal::i2c;
use esp_idf_hal::delay::Ets;
use esp_idf_sys as sys;
use mfrc522::Mfrc522;
use lcd_lcm1602_i2c::sync_lcd::Lcd;
use std::ffi::CString;
use std::thread;
use std::time::Duration;

use esp_idf_svc::wifi::{BlockingWifi, EspWifi, ClientConfiguration, Configuration};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::{EspHttpServer, Configuration as HttpConfig};

// Pulls these values from .env at compile time!
const WIFI_SSID: &str = match option_env!("WIFI_SSID") { Some(s) => s, None => "YOUR_WIFI_SSID" };
const WIFI_PASS: &str = match option_env!("WIFI_PASS") { Some(s) => s, None => "YOUR_WIFI_PASSWORD" };
const OLIVER_UID: &str = "4A085B7F";
const LEO_UID: &str = "8A9C617F";

const MOTDS: &[&str] = &[
    "Waltuh, put ur cup away waltuh",
    "I am the one who pours",
    "Kid named finger"
];

fn main() {
    sys::link_patches();

    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    init_nvs().expect("Failed to initialize NVS");

    let peripherals = esp_idf_hal::peripherals::Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // Wi-Fi Setup
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone())).unwrap(),
        sys_loop,
    ).unwrap();

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        password: WIFI_PASS.try_into().unwrap(),
        ..Default::default()
    })).unwrap();

    println!("Connecting to Wi-Fi...");
    if wifi.start().is_ok() && wifi.connect().is_ok() && wifi.wait_netif_up().is_ok() {
        println!("Connected to Wi-Fi!");
    } else {
        println!("Failed to connect to Wi-Fi. Continuing without it.");
    }

    // HTTP Server Setup
    let mut server = EspHttpServer::new(&HttpConfig::default()).unwrap();
    server.fn_handler("/", esp_idf_svc::http::Method::Get, |request| {
        let oliver = get_uid_count(OLIVER_UID).unwrap_or(0);
        let leo = get_uid_count(LEO_UID).unwrap_or(0);
        let html = format!(
            "<html><head><title>Moisture Leaderboard</title></head>\
            <body style='font-family: sans-serif; text-align: center; background-color: #1e1e2f; color: #ffffff;'>\
            <h1 style='color: #4da8da;'>Kid named finger</h1>\
            <h2>Leaderboard</h2>\
            <div style='font-size: 24px; margin: 20px;'>\
                <p>Oliver: <b>{}</b> taps</p>\
                <p>Leo: <b>{}</b> taps</p>\
            </div>\
            </body></html>",
            oliver, leo
        );
        let mut response = request.into_ok_response().unwrap();
        response.write(html.as_bytes()).unwrap();
        Ok::<(), sys::EspError>(())
    }).unwrap();

    // Reset endpoint - clears all tap counts
    server.fn_handler("/reset/password/waltuh", esp_idf_svc::http::Method::Get, |request| {
        reset_all_counts();
        let html = "<html><body style='font-family:sans-serif;text-align:center;background:#1e1e2f;color:#fff'>\
            <h1>Counts reset!</h1><a href='/'>Back</a></body></html>";
        let mut response = request.into_ok_response().unwrap();
        response.write(html.as_bytes()).unwrap();
        Ok::<(), sys::EspError>(())
    }).unwrap();

    println!("Web server running on port 80 - find IP in router DHCP table");

    // Configure I2C for LCD (using standard SDA=21, SCL=22)
    let i2c_config = i2c::config::Config::new().baudrate(100_000.into());
    let mut i2c = i2c::I2cDriver::new(peripherals.i2c0, pins.gpio21, pins.gpio22, &i2c_config).unwrap();
    let mut delay = Ets;
    
    // Try 0x27 first (most common), fall back to 0x3F
    let mut lcd_opt = Lcd::new(&mut i2c, &mut delay)
        .with_address(0x27)
        .with_rows(1)
        .init()
        .or_else(|_| {
            Lcd::new(&mut i2c, &mut delay)
                .with_address(0x3F)
                .with_rows(1)
                .init()
        })
        .map(|l| {
            println!("LCD initialized OK");
            Some(l)
        })
        .unwrap_or_else(|_| {
            println!("Failed to initialize LCD (tried 0x27 and 0x3F)");
            None
        });

    if let Some(ref mut lcd) = lcd_opt {
        let _ = lcd.clear();
        let _ = lcd.set_cursor(0, 0);
        let _ = lcd.write_str("Reader Ready");
    }

    // Configure SPI for MFRC522
    let spi = peripherals.spi2;
    let sclk = pins.gpio18;
    let serial_in = pins.gpio19;
    let serial_out = pins.gpio23;
    let cs = pins.gpio5;

    let spi_config = spi::config::Config::new().baudrate(1_000_000.into()).data_mode(spi::config::MODE_0);
    let spi_device = spi::SpiDeviceDriver::new_single(
        spi, sclk, serial_out, Some(serial_in), Some(cs),
        &spi::config::DriverConfig::new(), &spi_config,
    ).unwrap();

    let spi_itf = mfrc522::comm::blocking::spi::SpiInterface::new(spi_device);
    let mut mfrc522 = Mfrc522::new(spi_itf).init().unwrap();
    mfrc522.set_antenna_gain(mfrc522::RxGain::DB48).unwrap();

    println!("RFID Reader initialized. Waiting for cards...");

    loop {
        // Update LCD with current counts if idle
        if let Some(ref mut lcd) = lcd_opt {
            let oliver = get_uid_count(OLIVER_UID).unwrap_or(0);
            let leo = get_uid_count(LEO_UID).unwrap_or(0);
            let _ = lcd.clear();
            let _ = lcd.set_cursor(0, 0);
            let _ = lcd.write_str(&format!("Oli:{} Leo:{}", oliver, leo));
        }

        // We poll the reader for a few seconds before refreshing LCD
        for _ in 0..10 {
            if let Ok(atqa) = mfrc522.reqa() {
                if let Ok(uid) = mfrc522.select(&atqa) {
                    let uid_hex = format_uid_nodash(uid.as_bytes());
                    println!("Card tapped: {}", uid_hex);

                    if uid_hex == OLIVER_UID || uid_hex == LEO_UID {
                        let name = if uid_hex == OLIVER_UID { "Oliver" } else { "Leo" };
                        
                        match increment_uid_count(&uid_hex) {
                            Ok(new_count) => {
                                println!("{} tapped! New count: {}", name, new_count);
                                
                                if let Some(ref mut lcd) = lcd_opt {
                                    let _ = lcd.clear();
                                    let _ = lcd.set_cursor(0, 0);
                                    let _ = lcd.write_str(&format!("Thanks {}!", name));
                                    
                                    // Let them see "Thanks" for 1 second
                                    sleep_ms(1000);
                                    
                                    let r = unsafe { sys::esp_random() } as usize;
                                    let motd = MOTDS[r % MOTDS.len()];
                                    
                                    let _ = lcd.clear();
                                    let _ = lcd.set_cursor(0, 0);
                                    
                                    // Text wrap up to 32 chars across 2 lines
                                    let line1_len = if motd.len() > 16 { 16 } else { motd.len() };
                                    let _ = lcd.write_str(&motd[..line1_len]);
                                    
                                    if motd.len() > 16 {
                                        let _ = lcd.set_cursor(1, 0);
                                        let line2_len = if motd.len() > 32 { 32 } else { motd.len() };
                                        let _ = lcd.write_str(&motd[16..line2_len]);
                                    }
                                } else {
                                    // If no LCD, just wait the 2 seconds
                                    sleep_ms(1000);
                                }

                                // 5s cooldown
                                println!("Starting 5s cooldown...");
                                sleep_ms(5000);
                            },
                            Err(e) => println!("Failed to update NVS: {:?}", e),
                        }
                    } else {
                        println!("Unknown card: {}", uid_hex);
                        if let Some(ref mut lcd) = lcd_opt {
                            let _ = lcd.clear();
                            let _ = lcd.set_cursor(0, 0);
                            let _ = lcd.write_str("Unknown Card");
                        }
                        thread::sleep(Duration::from_secs(2));
                    }
                    
                    let _ = mfrc522.hlta();
                    break;
                }
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

fn format_uid_nodash(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join("")
}

/// Sleep in 100ms chunks so FreeRTOS IDLE always gets CPU time within the WDT window.
fn sleep_ms(ms: u64) {
    let chunks = ms / 100;
    let remainder = ms % 100;
    for _ in 0..chunks {
        thread::sleep(Duration::from_millis(100));
    }
    if remainder > 0 {
        thread::sleep(Duration::from_millis(remainder));
    }
}

/// Set both tap counts to zero in NVS.
fn reset_all_counts() {
    for uid in &[OLIVER_UID, LEO_UID] {
        let namespace = CString::new("storage").unwrap();
        let key = CString::new(*uid).unwrap();
        let mut handle: sys::nvs_handle_t = 0;
        unsafe {
            let err = sys::nvs_open(namespace.as_ptr(), sys::nvs_open_mode_t_NVS_READWRITE, &mut handle);
            if err == sys::ESP_OK {
                sys::nvs_set_u32(handle, key.as_ptr(), 0);
                sys::nvs_commit(handle);
                sys::nvs_close(handle);
            }
        }
    }
    println!("All tap counts reset to 0");
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

fn get_uid_count(uid_key: &str) -> Result<u32, sys::esp_err_t> {
    let namespace = CString::new("storage").expect("valid");
    let key = CString::new(uid_key).expect("valid");
    let mut handle = 0;
    let mut current = 0;
    unsafe {
        let err = sys::nvs_open(namespace.as_ptr(), sys::nvs_open_mode_t_NVS_READONLY, &mut handle);
        if err == sys::ESP_OK {
            sys::nvs_get_u32(handle, key.as_ptr(), &mut current);
            sys::nvs_close(handle);
        }
    }
    Ok(current)
}

fn increment_uid_count(uid_key: &str) -> Result<u32, sys::esp_err_t> {
    let namespace = CString::new("storage").expect("valid");
    let key = CString::new(uid_key).expect("valid");

    let mut handle: sys::nvs_handle_t = 0;
    let open_err = unsafe {
        sys::nvs_open(namespace.as_ptr(), sys::nvs_open_mode_t_NVS_READWRITE, &mut handle)
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