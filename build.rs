fn main() {
    // Tell cargo to re-compile if .env changes
    println!("cargo:rerun-if-changed=.env");

    // Load .env and expose variables to rustc
    if let Ok(_) = dotenvy::dotenv() {
        for (key, value) in std::env::vars() {
            println!("cargo:rustc-env={}={}", key, value);
        }
    }

    // Let embuild export ESP-IDF env vars for esp-idf-sys build scripts.
    embuild::espidf::sysenv::output();
}
