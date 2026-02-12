#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{
    cell::RefCell,
    fs::File,
    io::BufReader,
    net::IpAddr,
    rc::Rc,
    sync::mpsc::{Receiver, TryRecvError},
    time::Instant,
};

use fltk::{
    app,
    button::{self},
    dialog::NativeFileChooser,
    enums::CallbackTrigger,
    group::Flex,
    input,
    misc::Progress,
    prelude::{ButtonExt, DisplayExt, GroupExt, InputExt, WidgetBase, WidgetExt, WindowExt},
    text,
    window::Window,
};
use fltk_theme::{ColorTheme, color_themes};

#[cfg(target_os = "windows")]
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

mod downloader;
mod mmdb;

use ini::Ini;

enum Message {
    SendQuery(String),
    ShowMetadata,
    SaveConfig,
    ReloadConfig,
}

fn show_progress_modal<T: 'static, F, C>(
    title: &str,
    rx: Receiver<T>,
    to_progress: F,
    on_complete: C,
) where
    F: Fn(&T) -> Option<(f64, String)> + 'static,
    C: FnOnce(T) + 'static,
{
    let mut diag_win = Window::default().with_size(300, 60).with_label(title);
    diag_win.make_modal(true);

    let mut progress = Progress::new(0, 0, 300, 60, "");
    progress.set_minimum(0.0);
    progress.set_maximum(100.0);
    progress.set_selection_color(fltk::enums::Color::Blue);
    diag_win.end();
    diag_win.show();

    let mut on_complete = Some(on_complete);

    app::add_idle3(move |handle| match rx.try_recv() {
        Ok(event) => match to_progress(&event) {
            Some((p, label)) => {
                if progress.value() as u64 != (p * 100.0) as u64 {
                    progress.set_value(p * 100.0);
                    progress.set_label(&format!("{label} ({:.0}%)", p * 100.0));
                }
            }
            None => {
                diag_win.hide();
                if let Some(callback) = on_complete.take() {
                    callback(event);
                }
                app::remove_idle3(handle);
            }
        },
        Err(TryRecvError::Disconnected) => {
            diag_win.hide();
            app::remove_idle3(handle);
        }
        Err(TryRecvError::Empty) => {}
    });
}

fn get_config_path() -> std::path::PathBuf {
    let mut config_path = downloader::default_mmdb_path();
    config_path.set_file_name("config.ini");
    config_path
}

fn load_config() -> Option<String> {
    let config_path = get_config_path();
    if config_path.exists() {
        match Ini::load_from_file(&config_path) {
            Ok(ini) => {
                if let Some(section) = ini.section(Some("database")) {
                    section.get("path").map(|s| s.to_string())
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    } else {
        None
    }
}

fn save_config(db_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = get_config_path();
    let mut ini = Ini::new();
    ini.with_section(Some("database")).set("path", db_path);

    ini.write_to_file(&config_path)?;
    Ok(())
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Base);
    let theme = ColorTheme::new(color_themes::DARK_THEME);
    theme.apply();
    let mut wind = Window::default()
        .with_size(800, 600)
        .with_label("IP Info GUI");

    let mmdb = Rc::new(RefCell::new(None));
    let last_query_time = Rc::new(RefCell::new(0u64));

    let (s, r) = app::channel::<Message>();

    let mut col = Flex::default_fill().column();
    let mut row = Flex::default().row();
    let mut input_bar = input::Input::default();
    col.set_margins(10, 10, 10, 10);
    {
        input_bar.set_trigger(CallbackTrigger::EnterKeyAlways);
        input_bar.set_tooltip("IP address");

        let mut button = button::Button::default().with_label("Query");
        row.fixed(&button, 100);
        row.end();
        col.fixed(&row, 30);

        input_bar.set_callback(move |i| {
            let val = i.value();
            s.send(Message::SendQuery(val.to_string()));
        });

        let input_bar = input_bar.clone();
        button.set_callback(move |_| {
            let val = input_bar.value();
            s.send(Message::SendQuery(val.to_string()));
        });
    }

    let mut buffer = text::TextBuffer::default();

    let mut db_input_bar: input::Input;
    {
        let mut row = Flex::default().row();
        let input_label = fltk::frame::Frame::default().with_label("MMDB database:");
        let mut input_bar = input::Input::default();
        input_bar.set_trigger(CallbackTrigger::all());
        row.fixed(&input_label, 115);

        let mut browse_button = button::Button::default().with_label("...");
        row.fixed(&browse_button, 30);

        let mut download_btn = button::Button::default().with_label("Download default");
        row.fixed(&download_btn, 150);

        row.end();
        col.fixed(&row, 30);

        let mut input_bar2 = input_bar.clone();
        browse_button.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(fltk::dialog::FileDialogType::BrowseFile);
            chooser.set_filter("MMDB files\t*.mmdb");
            chooser.show();
            let path = chooser.filename();
            if !path.as_os_str().is_empty() {
                input_bar2.set_value(&path.to_string_lossy());
                input_bar2.do_callback();
            }
        });

        let btn = download_btn.clone();
        let row = row.clone();
        let input_bar = input_bar.clone();
        let mut input_bar2 = input_bar.clone();
        let mmdb = mmdb.clone();
        download_btn.set_callback(move |_| {
            let (rx, _handle) = downloader::download_default_mmdb();

            let mut btn = btn.clone();
            let row = row.clone();
            let mut input_bar = input_bar.clone();
            show_progress_modal(
                "Downloading...",
                rx,
                |event| match event {
                    downloader::DownloadEvent::Progress(p) => {
                        Some((*p, "Downloading...".to_string()))
                    }
                    downloader::DownloadEvent::Done(_) => None,
                },
                move |event| {
                    if let downloader::DownloadEvent::Done(path_buf) = event {
                        btn.hide();
                        row.recalc();
                        input_bar.set_value(path_buf.to_str().unwrap());
                    }
                },
            );
        });
        let mut buffer = buffer.clone();
        let mmdb = mmdb.clone();
        input_bar2.set_callback(move |i| {
            *mmdb.borrow_mut() = None;
            let value = i.value();
            if std::fs::exists(&value).unwrap_or(false) {
                let Ok(file) = File::open(&value) else {
                    buffer.set_text("Failed to load database (error during file open)");
                    return;
                };
                let file = BufReader::new(file);
                let new_mmdb = match mmdb::Mmdb::new(file) {
                    Ok(mmdb) => mmdb,
                    Err(err) => {
                        buffer.set_text(&format!("Error while reading database: {err:?}"));
                        return;
                    }
                };
                *mmdb.borrow_mut() = Some(new_mmdb);
                match (*mmdb.borrow_mut()).as_mut().unwrap().get_metadata_string() {
                    Ok(metadata) => {
                        buffer.set_text(&metadata);
                    }
                    Err(err) => {
                        buffer.set_text(&format!(
                            "Database loaded, but error getting metadata: {err:?}"
                        ));
                    }
                }
            } else {
                buffer.set_text(&format!("File does not exist '{value}'"));
            }
        });

        let config_db_path = load_config();
        let db_to_load = match config_db_path {
            Some(path) if std::path::Path::new(&path).exists() => {
                download_btn.hide();
                path
            }
            _ => {
                if downloader::default_mmdb_exists() {
                    download_btn.hide();
                    input_bar2.set_value(downloader::default_mmdb_path().to_str().unwrap());
                    input_bar2.do_callback();
                    downloader::default_mmdb_path()
                        .to_string_lossy()
                        .to_string()
                } else {
                    String::new()
                }
            }
        };

        if !db_to_load.is_empty() {
            input_bar2.set_value(&db_to_load);
            input_bar2.do_callback();
        }

        db_input_bar = input_bar2.clone();
    }

    {
        let mut row = Flex::default().row();

        let mut metadata_button = button::Button::default().with_label("Show DB Metadata");
        row.fixed(&metadata_button, 150);

        let mut save_config_button = button::Button::default().with_label("Save Config");
        row.fixed(&save_config_button, 120);

        let mut reload_config_button = button::Button::default().with_label("Reload Config");
        row.fixed(&reload_config_button, 120);

        let mut db_loaded_checkbox = button::CheckButton::default().with_label("Database loaded");
        db_loaded_checkbox.set_value(false);
        db_loaded_checkbox.deactivate();
        row.fixed(&db_loaded_checkbox, 140);

        let query_time_label = fltk::frame::Frame::default().with_label("Query time (ns):");
        row.fixed(&query_time_label, 120);

        let mut query_time_input = input::Input::default();
        query_time_input.set_value("0");
        query_time_input.deactivate();
        row.fixed(&query_time_input, 80);

        row.end();
        col.fixed(&row, 30);

        let s = s.clone();
        metadata_button.set_callback(move |_| {
            s.send(Message::ShowMetadata);
        });

        let s = s.clone();
        save_config_button.set_callback(move |_| {
            s.send(Message::SaveConfig);
        });

        let s = s.clone();
        reload_config_button.set_callback(move |_| {
            s.send(Message::ReloadConfig);
        });

        let mmdb = mmdb.clone();
        let mut db_loaded_checkbox = db_loaded_checkbox.clone();
        let mut metadata_button = metadata_button.clone();
        let mut save_config_button = save_config_button.clone();
        let _reload_config_button = reload_config_button.clone();
        let last_query_time = last_query_time.clone();
        let mut query_time_input = query_time_input.clone();

        app::add_idle3(move |_handle| {
            let is_loaded = mmdb.borrow().is_some();
            let current_state = db_loaded_checkbox.is_checked();
            if is_loaded != current_state {
                db_loaded_checkbox.set_value(is_loaded);
            }

            if is_loaded {
                metadata_button.activate();
                save_config_button.activate();
            } else {
                metadata_button.deactivate();
                save_config_button.deactivate();
            }

            let current_time = *last_query_time.borrow();
            let displayed_time = query_time_input.value().parse::<u64>().unwrap_or(0);
            if current_time != displayed_time {
                query_time_input.set_value(&current_time.to_string());
            }
        });
    }

    let mut display = text::TextDisplay::default();
    display.set_buffer(buffer.clone());

    col.end();
    wind.end();
    wind.show();

    #[cfg(target_os = "windows")]
    unsafe {
        use fltk::prelude::WindowExt;
        let hwnd = wind.raw_handle();
        let dark_mode: i32 = 1;

        for attr in [20u32, 19u32] {
            windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                hwnd as _,
                attr, // Pass the u32 attribute ID
                &dark_mode as *const i32 as _,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    while app.wait() {
        if let Some(msg) = r.recv() {
            match msg {
                Message::SaveConfig => {
                    if let Some(_mmdb) = mmdb.borrow().as_ref() {
                        let db_path = db_input_bar.value();
                        if !db_path.is_empty() && std::path::Path::new(&db_path).exists() {
                            match save_config(&db_path) {
                                Ok(()) => {
                                    buffer.set_text("Configuration saved successfully");
                                }
                                Err(err) => {
                                    buffer.set_text(&format!("Error saving configuration: {err}"));
                                }
                            }
                        } else {
                            buffer.set_text("Cannot save config: no valid database path");
                        }
                    } else {
                        buffer.set_text("Cannot save config: no database loaded");
                    }
                }
                Message::ReloadConfig => {
                    let config_db_path = load_config();
                    match config_db_path {
                        Some(path) if std::path::Path::new(&path).exists() => {
                            db_input_bar.set_value(&path);
                            db_input_bar.do_callback();
                        }
                        Some(_) => {
                            buffer.set_text("Configured database path does not exist");
                        }
                        None => {
                            buffer.set_text("No configuration found");
                        }
                    }
                }
                Message::ShowMetadata => {
                    if let Some(mmdb) = mmdb.borrow_mut().as_mut() {
                        match mmdb.get_metadata_string() {
                            Ok(metadata) => {
                                buffer.set_text(&metadata);
                                display.set_insert_position(buffer.length());
                                display.show_insert_position();
                            }
                            Err(err) => {
                                buffer.set_text(&format!("Error getting metadata: {err:?}"));
                            }
                        }
                    } else {
                        buffer.set_text("No database loaded to show metadata");
                    }
                }
                Message::SendQuery(msg) => {
                    if let Some(_mmdb_ref) = mmdb.borrow_mut().as_mut() {
                        let is_domain = msg.parse::<IpAddr>().is_err();

                        if is_domain {
                            let (tx, rx) = std::sync::mpsc::channel::<mmdb::QueryProgress>();
                            let (result_tx, result_rx) =
                                std::sync::mpsc::channel::<Result<IpAddr, mmdb::MmdbError>>();

                            let msg_clone = msg.clone();
                            let tx_clone = tx.clone();

                            std::thread::spawn(move || {
                                let _ = tx_clone.send(mmdb::QueryProgress::Started);

                                match mmdb::dns::query_dns_for_domain(&msg_clone) {
                                    Ok(ip) => {
                                        let _ = tx_clone.send(mmdb::QueryProgress::DnsResolved);
                                        let _ = result_tx.send(Ok(ip));
                                        let _ = tx_clone.send(mmdb::QueryProgress::Completed);
                                    }
                                    Err(e) => {
                                        let _ = result_tx.send(Err(mmdb::MmdbError::DnsError(e)));
                                        let _ = tx_clone.send(mmdb::QueryProgress::Completed);
                                    }
                                }
                            });

                            let mmdb = mmdb.clone();
                            let mut buffer = buffer.clone();
                            let last_query_time = last_query_time.clone();
                            let msg = msg.clone();

                            show_progress_modal(
                                "Resolving DNS...",
                                rx,
                                |event| {
                                    event
                                        .to_progress()
                                        .map(|x| (x, "Resolving domain...".to_string()))
                                },
                                move |_final_event| {
                                    if let Ok(Ok(ip)) = result_rx.try_recv() {
                                        let start_time = Instant::now();

                                        if let Some(mmdb_ref) = mmdb.borrow_mut().as_mut() {
                                            let result = mmdb_ref.query_ip(ip);
                                            let elapsed_ns = start_time.elapsed().as_nanos() as u64;
                                            *last_query_time.borrow_mut() = elapsed_ns;

                                            match result {
                                                Ok(data) => {
                                                    let mut output = String::new();
                                                    output.push_str(&format!(
                                                        "DNS: Resolved domain '{}' -> {}\n",
                                                        msg, ip
                                                    ));
                                                    match data {
                                                        Some(res) => {
                                                            output.push_str(&format!("{res}"));
                                                        }
                                                        None => {
                                                            output.push_str(&format!(
                                                                "No data found for IP '{ip}'"
                                                            ));
                                                        }
                                                    }
                                                    buffer.set_text(&output);
                                                }
                                                Err(err) => {
                                                    buffer.set_text(&format!(
                                                        "Error during query '{err:?}'"
                                                    ));
                                                }
                                            }
                                        }
                                    } else if let Ok(Err(e)) = result_rx.try_recv() {
                                        buffer.set_text(&format!(
                                            "Error during DNS resolution: {e:?}"
                                        ));
                                    }
                                },
                            );
                        } else {
                            buffer.set_text("");
                            let start_time = Instant::now();
                            let result = _mmdb_ref.query_string(&msg);
                            let elapsed_ns = start_time.elapsed().as_nanos() as u64;
                            *last_query_time.borrow_mut() = elapsed_ns;

                            match result {
                                Ok(info) => {
                                    let mut output = String::new();
                                    if let Some(dns_info) = &info.dns_info {
                                        output.push_str(&format!(
                                            "DNS: Resolved domain '{}' -> {}\n",
                                            dns_info.domain, dns_info.resolved_ip
                                        ));
                                    }
                                    match info.data {
                                        Some(res) => {
                                            output.push_str(&format!("{res}"));
                                        }
                                        None => {
                                            output
                                                .push_str(&format!("No data found for IP '{msg}'"));
                                        }
                                    }
                                    buffer.set_text(&output);
                                    display.set_insert_position(buffer.length());
                                    display.show_insert_position();
                                }
                                Err(err) => {
                                    buffer.set_text(&format!("Error during query '{err:?}'"));
                                }
                            }
                        }
                    } else {
                        buffer.set_text("Failed to query IP as database is not loaded");
                    }

                    let val = input_bar.value();
                    input_bar.take_focus().unwrap();
                    input_bar.set_position(val.len() as i32).unwrap();
                    input_bar.set_mark(0).unwrap();
                }
            }
        }
    }
}
