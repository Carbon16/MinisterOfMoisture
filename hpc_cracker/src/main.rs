use rayon::prelude::*;
use clap::Parser;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    uid: String,
    #[arg(short, long)]
    nt: String,
    #[arg(short, long)]
    nr: String,
    #[arg(short, long)]
    ar: String,
}

struct Crypto1 {
    state: u64,
}

impl Crypto1 {
    fn new(key: u64, uid: u32) -> Self {
        let mut s = Self { state: 0 };
        // Initializing with key and uid
        for i in 0..48 {
            let bit = ((key >> i) & 1) ^ ((uid as u64 >> (i % 32)) & 1);
            s.feed(bit as u8, false);
        }
        s
    }

    fn feed(&mut self, in_bit: u8, out_bit: u8) {
        let feedback = ((self.state >> 0) ^ (self.state >> 5) ^ (self.state >> 9) ^ (self.state >> 10) ^
                       (self.state >> 12) ^ (self.state >> 14) ^ (self.state >> 15) ^ (self.state >> 17) ^
                       (self.state >> 19) ^ (self.state >> 24) ^ (self.state >> 25) ^ (self.state >> 27) ^
                       (self.state >> 29) ^ (self.state >> 35) ^ (self.state >> 39) ^ (self.state >> 41) ^
                       (self.state >> 42) ^ (self.state >> 43)) & 1;
        self.state = (self.state >> 1) | ((feedback ^ in_bit as u64 ^ out_bit as u64) << 47);
    }

    fn bit(&mut self, in_bit: u8) -> u8 {
        let keystream = self.filter();
        self.feed(in_bit, keystream);
        keystream
    }

    fn filter(&self) -> u8 {
        let s = self.state;
        let mut f = ((s >> 9) & 1) ^ ((s >> 11) & 1) ^ ((s >> 13) & 1) ^ ((s >> 15) & 1);
        f ^= ((s >> 17) & 1) & ((s >> 19) & 1) & ((s >> 21) & 1) & ((s >> 23) & 1);
        f as u8
    }

    fn word(&mut self, data: u32) -> u32 {
        let mut res = 0;
        for i in 0..32 {
            let bit = self.bit(((data >> i) & 1) as u8);
            res |= (bit as u32) << i;
        }
        res
    }
}

fn main() {
    let args = Args::parse();
    
    let uid = u32::from_str_radix(&args.uid.trim_start_matches("0x"), 16).expect("Invalid UID");
    let nt = u32::from_str_radix(&args.nt.trim_start_matches("0x"), 16).expect("Invalid nt");
    let nr = u32::from_str_radix(&args.nr.trim_start_matches("0x"), 16).expect("Invalid nr");
    let target_ar = u32::from_str_radix(&args.ar.trim_start_matches("0x"), 16).expect("Invalid ar");
    
    println!("--- HPC CRACKER START ---");
    println!("UID: {:08X} | nt: {:08X} | nr: {:08X} | ar: {:08X}", uid, nt, nr, target_ar);

    let start_time = Instant::now();
    
    // We parallelize the search across the 48-bit keyspace
    let result: Option<u64> = (0..0x10000u64).into_par_iter().find_map_any(|high| {
        let base = high << 32;
        for low in 0..0xFFFFFFFFu32 {
            let key = base | (low as u64);
            if check_key(key, uid, nt, nr, target_ar) {
                return Some(key);
            }
        }
        None
    });

    if let Some(key) = result {
        println!("\n[!!!] KEY FOUND: {:012X}", key);
        println!("Time elapsed: {:?}", start_time.elapsed());
    } else {
        println!("\nSearch complete. No key found.");
    }
}

fn check_key(key: u64, uid: u32, nt: u32, nr: u32, target_ar: u32) -> bool {
    let mut c = Crypto1::new(key, uid);
    
    // Feed in Card Nonce
    for i in 0..32 { c.bit(((nt >> i) & 1) as u8); }
    
    // Authenticate Reader Nonce and verify Response
    let ks = c.word(nr);
    // ar is the bit-swapped successor of nt XORed with keystream
    // This is the core check
    if ks ^ target_ar == 0 { // Simplified check for demonstration
        return true;
    }
    false
}
