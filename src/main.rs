// Prelude removed in 0.44+
use esp_idf_hal::spi;
// use esp_idf_hal::i2c;       // LCD disabled
// use esp_idf_hal::delay::Ets; // LCD disabled
use esp_idf_sys as sys;
use mfrc522::Mfrc522;
// use lcd_lcm1602_i2c::sync_lcd::Lcd; // LCD disabled
use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::{Configuration as HttpConfig, EspHttpServer};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{BlockingWifi, ClientConfiguration, Configuration, EspWifi};

// Pulls these values from .env at compile time!
const WIFI_SSID: &str = match option_env!("WIFI_SSID") {
    Some(s) => s,
    None => "YOUR_WIFI_SSID",
};
const WIFI_PASS: &str = match option_env!("WIFI_PASS") {
    Some(s) => s,
    None => "YOUR_WIFI_PASSWORD",
};
const OLIVER_UID: &str = "4A085B7F";
const LEO_UID: &str = "8A9C617F";

const MOTDS: &[&str] = &[
    "Waltuh, put ur cup away",
    "I am the one who pours",
    "Kid named finger",
    "Say my name",
    "I am the danger",
];

fn main() {
    sys::link_patches();

    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    init_nvs().expect("Failed to initialize NVS");

    let peripherals = esp_idf_hal::peripherals::Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // Wi-Fi Setup (Core 0)
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone())).unwrap(),
        sys_loop,
    )
    .unwrap();

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        password: WIFI_PASS.try_into().unwrap(),
        ..Default::default()
    }))
    .unwrap();

    println!("Connecting to Wi-Fi...");
    if wifi.start().is_ok() && wifi.connect().is_ok() && wifi.wait_netif_up().is_ok() {
        println!("Connected to Wi-Fi!");
    } else {
        println!("Failed to connect to Wi-Fi — rebooting in 3s...");
        sleep_ms(3000);
        unsafe {
            sys::esp_restart();
        }
    }

    // Load initial counts from NVS into atomics
    // These live on the heap and are shared via Arc — no locks needed
    let oliver_count = Arc::new(AtomicU32::new(get_uid_count(OLIVER_UID).unwrap_or(0)));
    let leo_count = Arc::new(AtomicU32::new(get_uid_count(LEO_UID).unwrap_or(0)));

    // Clone Arcs for HTTP handlers
    let oliver_http = oliver_count.clone();
    let leo_http = leo_count.clone();
    let oliver_reset = oliver_count.clone();
    let leo_reset = leo_count.clone();

    // HTTP Server (runs its own FreeRTOS tasks internally — no blocking on Core 0)
    let mut server = EspHttpServer::new(&HttpConfig::default()).unwrap();

    server
        .fn_handler("/", esp_idf_svc::http::Method::Get, move |request| {
            // Read directly from atomics — no NVS, no blocking
            let oliver = oliver_http.load(Ordering::Relaxed);
            let leo = leo_http.load(Ordering::Relaxed);
            let html = format!(
                "<html><head><title>Moisture Leaderboard</title></head>\
            <body style='font-family: sans-serif; text-align: center; \
                         background-color: #1e1e2f; color: #ffffff;'>\
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
        })
        .unwrap();

    server
        .fn_handler(
            "/reset/password/waltuh",
            esp_idf_svc::http::Method::Get,
            move |request| {
                // Reset atomics AND NVS
                oliver_reset.store(0, Ordering::Relaxed);
                leo_reset.store(0, Ordering::Relaxed);
                let _ = set_uid_count(OLIVER_UID, 0);
                let _ = set_uid_count(LEO_UID, 0);
                println!("All tap counts reset to 0");
                let html = "<html><body style='font-family:sans-serif;text-align:center;\
            background:#1e1e2f;color:#fff'>\
            <h1>Counts reset!</h1><a href='/'>Back</a></body></html>";
                let mut response = request.into_ok_response().unwrap();
                response.write(html.as_bytes()).unwrap();
                Ok::<(), sys::EspError>(())
            },
        )
        .unwrap();

    println!("Web server running on port 80");

    // LCD disabled
    // let i2c_config = i2c::config::Config::new().baudrate(100_000.into());
    // let mut i2c = i2c::I2cDriver::new(peripherals.i2c0, pins.gpio21, pins.gpio22, &i2c_config).unwrap();

    // Configure SPI for MFRC522 — these get MOVED into the RFID thread
    let spi = peripherals.spi2;
    let sclk = pins.gpio18;
    let serial_in = pins.gpio19;
    let serial_out = pins.gpio23;
    let cs = pins.gpio5;

    // Clone Arcs for RFID thread
    let oliver_rfid = oliver_count.clone();
    let leo_rfid = leo_count.clone();

    // -----------------------------------------------------------------------
    // RFID thread — pinned to Core 1 via FreeRTOS scheduler
    // The main thread (Core 0) is now completely free for HTTP + IDLE
    // -----------------------------------------------------------------------
    let _rfid_thread = thread::Builder::new()
        .stack_size(32768)
        .spawn(move || {
            let spi_config = spi::config::Config::new()
                .baudrate(1_000_000.into())
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
            .unwrap();

            let spi_itf = mfrc522::comm::blocking::spi::SpiInterface::new(spi_device);
            let mut mfrc522 = Mfrc522::new(spi_itf).init().unwrap();
            mfrc522.set_antenna_gain(mfrc522::RxGain::DB48).unwrap();

            println!("RFID Reader initialized on Core 1. Waiting for cards...");

            loop {
                match mfrc522.reqa() {
                    Ok(atqa) => {
                        if let Ok(uid) = mfrc522.select(&atqa) {
                            let uid_hex = format_uid_nodash(uid.as_bytes());
                            println!("Card tapped: {}", uid_hex);

                            let name_and_atom: Option<(&str, &Arc<AtomicU32>)> =
                                if uid_hex == OLIVER_UID {
                                    Some(("Oliver", &oliver_rfid))
                                } else if uid_hex == LEO_UID {
                                    Some(("Leo", &leo_rfid))
                                } else {
                                    None
                                };

                            if let Some((name, atom)) = name_and_atom {
                                // Atomic increment — visible to HTTP handlers instantly
                                let new_count = atom.fetch_add(1, Ordering::Relaxed) + 1;
                                // Persist to NVS for power-loss safety
                                let _ = set_uid_count(
                                    if name == "Oliver" {
                                        OLIVER_UID
                                    } else {
                                        LEO_UID
                                    },
                                    new_count,
                                );
                                println!("{} tapped! New count: {}", name, new_count);

                                let r = unsafe { sys::esp_random() } as usize;
                                println!("MOTD: {}", MOTDS[r % MOTDS.len()]);

                                // Cooldown — yields Core 1 IDLE the whole time
                                sleep_ms(5000);
                            } else {
                                println!("Unknown card: {}", uid_hex);
                                sleep_ms(2000);
                            }

                            let _ = mfrc522.hlta();
                        }
                    }
                    Err(_) => {
                        // No card present — yield for 200ms so Core 1 IDLE runs
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        })
        .unwrap();

    // Core 0 main loop — does nothing except keep wifi/server alive.
    // The HTTP server already spawns its own FreeRTOS tasks.
    // IDLE0 runs freely here.
    println!("Main loop idle on Core 0. RFID polling on Core 1.");
    loop {
        sleep_ms(10_000);
        // Watchdog: reboot if Wi-Fi has dropped
        if !wifi.is_connected().unwrap_or(false) {
            println!("Wi-Fi dropped — rebooting...");
            sleep_ms(1000);
            unsafe {
                sys::esp_restart();
            }
        }
    }
}

fn format_uid_nodash(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join("")
}

/// Sleep in 100ms chunks — ensures the scheduler always gets a chance to run IDLE.
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
        let retry = unsafe { sys::nvs_flash_init() };
        if retry == sys::ESP_OK {
            Ok(())
        } else {
            Err(retry)
        }
    } else {
        Err(err)
    }
}

fn get_uid_count(uid_key: &str) -> Result<u32, sys::esp_err_t> {
    let namespace = CString::new("storage").expect("valid");
    let key = CString::new(uid_key).expect("valid");
    let mut handle: sys::nvs_handle_t = 0;
    let mut current: u32 = 0;
    unsafe {
        let err = sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READONLY,
            &mut handle,
        );
        if err == sys::ESP_OK {
            sys::nvs_get_u32(handle, key.as_ptr(), &mut current);
            sys::nvs_close(handle);
        }
    }
    Ok(current)
}

fn set_uid_count(uid_key: &str, value: u32) -> Result<(), sys::esp_err_t> {
    let namespace = CString::new("storage").expect("valid");
    let key = CString::new(uid_key).expect("valid");
    let mut handle: sys::nvs_handle_t = 0;
    unsafe {
        let err = sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut handle,
        );
        if err != sys::ESP_OK {
            return Err(err);
        }
        sys::nvs_set_u32(handle, key.as_ptr(), value);
        sys::nvs_commit(handle);
        sys::nvs_close(handle);
    }
    Ok(())
}
