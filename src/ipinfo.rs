use std::{
    io::{BufReader, Write},
    net::IpAddr,
    str::FromStr,
};

mod downloader;
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
            let db_path = downloader::default_mmdb_path();

            if !downloader::default_mmdb_exists() {
                eprintln!(
                    "ERR: ip address database does not exist (searching at {:?})",
                    db_path
                );
                eprint!(
                    "Automatically download database from '{}' (72.2MB)? y/n: ",
                    downloader::download_url()
                );
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
                let line = line.trim();
                if line.to_lowercase() != "y" {
                    eprintln!("Aborted");
                    return;
                }

                let mut pb = ProgressBar::default();

                let (rx, handle) = downloader::download_default_mmdb();
                for event in rx.iter() {
                    match event {
                        downloader::DownloadEvent::Progress(percent) => {
                            // println!("{percent}");
                            pb.set_progress(percent);
                        }
                        downloader::DownloadEvent::Done(_) => {
                            pb.finish();
                        }
                    }
                }
                drop(handle);
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
    percent: u8,
    width: u16,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self {
            width: 50,
            percent: 0,
        }
    }
}

impl ProgressBar {
    pub fn set_progress(&mut self, progress: f64) {
        let new_progress = (progress * 100.0) as u8;
        if new_progress != self.percent {
            self.percent = (progress * 100.0) as u8;
            self.draw();
        }
    }

    pub fn finish(&self) {
        eprint!("\r{}        ", " ".repeat(self.width as usize));
        let _ = std::io::stderr().lock().flush();
        eprintln!();
    }

    fn draw(&self) {
        let pct = self.percent as f64 / 100.0;
        let filled = (pct * self.width as f64) as u16;
        let empty = self.width - filled;

        eprint!(
            "\r[{}>{}] {:.0}% ",
            "=".repeat(filled as usize),
            " ".repeat(empty as usize),
            pct * 100.0,
        );
        let _ = std::io::stderr().lock().flush();
    }
}
