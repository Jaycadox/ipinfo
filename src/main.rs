use indicatif::{ProgressBar, ProgressStyle};
use std::{fs::File, io::BufReader, net::IpAddr, str::FromStr};

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
            let db_path = directories::ProjectDirs::from("xyz", "jayphen", "ipinfo")
                .expect("failed to get user directory");
            let db_path_parent = db_path.data_dir();
            let db_path = db_path_parent.join("db.mmdb");

            if !std::fs::exists(&db_path).unwrap_or(false) {
                eprintln!(
                    "ERR: ip address database does not exist (searching at {:?})",
                    db_path
                );
                eprint!(
                    "Automatically download database from 'https://github.com/iplocate/ip-address-databases/raw/d2264aeeffceb0ec401a05581a9401150a79eb5a/ip-to-asn/ip-to-asn.mmdb?download=true' (72.2MB)? y/n: "
                );
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
                let line = line.trim();
                if line != "y" {
                    eprintln!("Aborted");
                    return;
                }
                let mut resp = ureq::get("https://github.com/iplocate/ip-address-databases/raw/d2264aeeffceb0ec401a05581a9401150a79eb5a/ip-to-asn/ip-to-asn.mmdb?download=true").call().unwrap();

                let download_size = resp
                    .headers()
                    .get("Content-Length")
                    .and_then(|x| x.to_str().unwrap().parse::<u64>().ok())
                    .unwrap_or(0);

                let bar = ProgressBar::new(download_size);
                bar.set_style(ProgressStyle::default_bar()
                        .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})").unwrap()
                        .progress_chars("#>-"));

                let body = resp.body_mut();
                let mut source = bar.wrap_read(body.as_reader());
                std::fs::create_dir_all(db_path_parent).unwrap();
                let mut dest = File::create(&db_path).unwrap();

                std::io::copy(&mut source, &mut dest).unwrap();
                bar.finish_with_message("OK");
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
