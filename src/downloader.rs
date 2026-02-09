use std::{env, fs::File, io::BufWriter, path::PathBuf, thread::JoinHandle};

use byteorder::WriteBytesExt;

const URL: &str = "https://github.com/iplocate/ip-address-databases/raw/d2264aeeffceb0ec401a05581a9401150a79eb5a/ip-to-asn/ip-to-asn.mmdb?download=true";

#[allow(dead_code)]
pub enum DownloadEvent {
    Progress(f64),
    Done(PathBuf),
}
pub fn default_mmdb_exists() -> bool {
    std::fs::exists(default_mmdb_path()).unwrap_or(false)
}

#[allow(dead_code)]
pub fn download_url() -> &'static str {
    URL
}

pub fn download_default_mmdb() -> (std::sync::mpsc::Receiver<DownloadEvent>, JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<DownloadEvent>();
    let handle = std::thread::spawn(move || {
        let resp = tinyget::get(URL).send_lazy().unwrap();
        let path = default_mmdb_path();

        // let mut pb = ProgressBar::default();
        let mut writer = BufWriter::new(File::create(&path).unwrap());
        let mut total_bytes = None;

        for byte in resp {
            let (byte, len) = byte.unwrap();
            let total_bytes = match total_bytes {
                None => {
                    total_bytes = Some(len);
                    len
                }
                Some(len) => len,
            };

            writer.write_u8(byte).unwrap();
            tx.send(DownloadEvent::Progress(
                (total_bytes as f64 - len as f64) / total_bytes as f64,
            ))
            .unwrap();
            // pb.inc_and_set_remaining(len as u64);
        }
        tx.send(DownloadEvent::Done(path)).unwrap();
        // pb.finish();
    });

    (rx, handle)
}

pub fn default_mmdb_path() -> PathBuf {
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
