use std::{
    env,
    fs::File,
    io::{BufReader, BufWriter, Write},
    net::IpAddr,
    path::PathBuf,
    str::FromStr,
};

use byteorder::WriteBytesExt;

mod mmdb;

fn main() {
    let mut verbose = false;
    let mut args = std::env::args();
    let program = args.next().unwrap();
    let program = std::path::Path::new(&program);
    let program = program.file_name().unwrap().to_string_lossy();

    let args = args
        .filter(|x| {
            if x == "-v" || x == "--verbose" {
                verbose = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    if args.len() != 1 && args.len() != 2 {
        eprintln!("{program} -- locally query ip information via a MMDB database");
        eprintln!("USAGE: {program} <ip address> (mmdb_path)");
        eprintln!("   eg. {program} 1.1.1.1");
        eprintln!("   eg. {program} 1.1.1.1 ./ip_to_country.mmdb");
        eprintln!("FLAGS:");
        eprintln!("       --verbose (-v)      Enables verbose logging");
        eprintln!(
            "NOTE: the `mmdb_path` argument is optional, if not present, {program} can automatically download and use a default ip-to-asn mmdb database (provided by IPLocate.io)."
        );
        return;
    }

    let ip = args[0].clone();

    let Ok(ip) = ip.parse::<IpAddr>() else {
        eprintln!("ERR: the provided ip address '{ip}' is invalid");
        return;
    };

    let db_path = match args.get(1) {
        Some(arg) => std::path::PathBuf::from_str(arg).expect("invalid path"),
        None => {
            let db_path = get_db_path();

            const URL: &str = "https://github.com/iplocate/ip-address-databases/raw/d2264aeeffceb0ec401a05581a9401150a79eb5a/ip-to-asn/ip-to-asn.mmdb?download=true";

            if !std::fs::exists(&db_path).unwrap_or(false) {
                eprintln!(
                    "ERR: ip address database does not exist (searching at {:?})",
                    db_path
                );
                eprint!("Automatically download database from '{URL}' (72.2MB)? y/n: ");
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
                let line = line.trim();
                if line.to_lowercase() != "y" {
                    eprintln!("Aborted");
                    return;
                }
                let resp = tinyget::get(URL).send_lazy().unwrap();

                let mut pb = ProgressBar::default();
                let mut writer = BufWriter::new(File::create(&db_path).unwrap());

                for byte in resp {
                    let (byte, len) = byte.unwrap();
                    writer.write_u8(byte).unwrap();
                    pb.inc_and_set_remaining(len as u64);
                }
                pb.finish();
            }
            db_path
        }
    };

    let file = std::fs::File::open(db_path).unwrap();

    mmdb::set_verbose(verbose);

    let mut mmdb = mmdb::Mmdb::new(BufReader::new(file)).unwrap();
    let typ = mmdb.query_ip(ip).unwrap();

    match typ {
        Some(typ) => {
            println!("{typ}");
        }
        None => {
            println!("No data found");
        }
    }
}

pub struct ProgressBar {
    total: u64,
    current: u64,
    width: u16,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self {
            total: 0,
            current: 0,
            width: 50,
        }
    }
}

impl ProgressBar {
    pub fn inc_and_set_remaining(&mut self, new_total: u64) {
        self.current = self.current.saturating_add(1);
        self.total = new_total.saturating_add(self.current);
        if self.current.is_multiple_of(100000) {
            self.draw();
        }
    }

    pub fn finish(&self) {
        eprintln!();
    }

    fn draw(&self) {
        let pct = if self.total == 0 {
            1.0
        } else {
            self.current as f64 / self.total as f64
        };
        let filled = (pct * self.width as f64) as u16;
        let empty = self.width - filled;

        eprint!(
            "\r[{}>{}] {:.0}% ({}/{})             ",
            "=".repeat(filled as usize),
            " ".repeat(empty as usize),
            pct * 100.0,
            human_bytes(self.current),
            human_bytes(self.total),
        );
        let _ = std::io::stderr().lock().flush();
    }
}

fn human_bytes(b: u64) -> String {
    match b {
        0..1024 => format!("{b} B"),
        1024..1_048_576 => format!("{:.1} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

fn get_db_path() -> PathBuf {
    let mut base_dir = if cfg!(target_os = "windows") {
        // Windows: %APPDATA% (C:\Users\Name\AppData\Roaming)
        env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        // macOS: ~/Library/Application Support
        env::var_os("HOME").map(|h| {
            let mut p = PathBuf::from(h);
            p.push("Library");
            p.push("Application Support");
            p
        })
    } else {
        // Linux/Unix: $XDG_DATA_HOME or ~/.local/share
        env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| {
            env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".local/share");
                p
            })
        })
    }
    .expect("Could not determine config directory");

    base_dir.push("ipinfo");

    let _ = std::fs::create_dir_all(&base_dir);

    base_dir.push("db.mmdb");
    base_dir
}
