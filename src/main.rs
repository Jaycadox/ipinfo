use indicatif::{ProgressBar, ProgressStyle};
use std::{fs::File, io::BufReader, net::IpAddr};

mod mmdb;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        eprintln!("ipinfo");
        eprintln!("USAGE: ipinfo <ip address>");
        eprintln!("   eg. ipinfo 1.1.1.1");
        return;
    }

    let ip = args[0].clone();

    let Ok(ip) = ip.parse::<IpAddr>() else {
        eprintln!("ERR: the provided ip address '{ip}' is invalid");
        return;
    };

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

    let file = std::fs::File::open(db_path).unwrap();
    let mut mmdb = mmdb::Mmdb::new(BufReader::new(file)).unwrap();
    let typ = mmdb.query_ip(ip).unwrap();

    match typ {
        Some(typ) => {
            mmdb::pretty_print_type(&typ, 0);
        }
        None => {
            println!("No data found");
        }
    }
}
