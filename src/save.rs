//! saveToFile / loadFromFile: "%d %d" of seed and branch count.

use std::io::{Read, Write};

pub fn save_to_file(fname: &[u8], seed: i32, branch_count: i32, stdout_out: &mut Vec<u8>) -> bool {
    let path = std::str::from_utf8(fname).unwrap_or("");
    let file = std::fs::File::create(path);
    match file {
        Ok(mut f) => {
            let _ = write!(f, "{seed} {branch_count}");
            true
        }
        Err(_) => {
            stdout_out.extend_from_slice(
                format!("error: file was not opened properly for writing: {}\n", String::from_utf8_lossy(fname)).as_bytes(),
            );
            false
        }
    }
}

pub fn load_from_file(fname: &[u8], stdout_out: &mut Vec<u8>) -> Option<(i32, i32)> {
    let path = std::str::from_utf8(fname).unwrap_or("");
    let mut data = Vec::new();
    match std::fs::File::open(path) {
        Ok(mut f) => {
            if f.read_to_end(&mut data).is_err() {
                stdout_out.extend_from_slice(b"error: save file could not be read\n");
                return None;
            }
        }
        Err(_) => {
            stdout_out.extend_from_slice(
                format!("error: file was not opened properly for reading: {}\n", String::from_utf8_lossy(fname)).as_bytes(),
            );
            return None;
        }
    }
    // fscanf("%i %i"): two base-0 integers separated by whitespace
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut it = text.split_ascii_whitespace();
    let (a, b) = match (it.next(), it.next()) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            stdout_out.extend_from_slice(b"error: save file could not be read\n");
            return None;
        }
    };
    let seed = match c_scan_int(a.as_bytes()) {
        Some(v) => v,
        None => {
            stdout_out.extend_from_slice(b"error: save file could not be read\n");
            return None;
        }
    };
    let count = match c_scan_int(b.as_bytes()) {
        Some(v) => v,
        None => {
            stdout_out.extend_from_slice(b"error: save file could not be read\n");
            return None;
        }
    };
    Some((seed, count))
}

/// fscanf %i: optional sign, then 0x hex / 0 octal / decimal.
fn c_scan_int(s: &[u8]) -> Option<i32> {
    let (v, len) = crate::numparse::strtol_base(s, 0);
    if len == 0 {
        return None;
    }
    // trailing garbage is tolerated by %i as long as something parsed
    Some(v as i32)
}
