#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{cell::RefCell, fs::File, io::BufReader, net::IpAddr, rc::Rc, sync::mpsc::TryRecvError};

use fltk::{
    app,
    button::{self},
    dialog::NativeFileChooser,
    enums::CallbackTrigger,
    group::Flex,
    input,
    misc::Progress,
    prelude::{DisplayExt, GroupExt, InputExt, WidgetBase, WidgetExt, WindowExt},
    text,
    window::Window,
};
use fltk_theme::{ColorTheme, color_themes};

mod downloader;

enum Message {
    SendQuery(String),
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Base);
    let theme = ColorTheme::new(color_themes::DARK_THEME);
    theme.apply();
    let mut wind = Window::default()
        .with_size(800, 600)
        .with_label("IP Info GUI");

    let mmdb = Rc::new(RefCell::new(None));

    let (s, r) = app::channel::<Message>();

    let mut col = Flex::default_fill().column();
    col.set_margins(10, 10, 10, 10);
    {
        let mut row = Flex::default().row();

        let mut input_bar = input::Input::default();
        input_bar.set_trigger(CallbackTrigger::EnterKeyAlways);
        input_bar.set_tooltip("IP address");

        let mut button = button::Button::default().with_label("Query");
        row.fixed(&button, 100);
        row.end();
        col.fixed(&row, 30);

        input_bar.set_callback(move |i| {
            let val = i.value();
            s.send(Message::SendQuery(val.to_string()));
            i.take_focus().unwrap();
            i.set_position(val.len() as i32).unwrap();
            i.set_mark(0).unwrap();
        });

        button.set_callback(move |i| {
            let val = input_bar.value();
            s.send(Message::SendQuery(val.to_string()));
        });
    }

    let mut buffer = text::TextBuffer::default();
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
            let mut diag_win = Window::default()
                .with_size(300, 60)
                .with_label("Processing...");
            diag_win.make_modal(true);

            let mut progress = Progress::new(0, 0, 300, 60, "");
            progress.set_minimum(0.0);
            progress.set_maximum(100.0);
            progress.set_selection_color(fltk::enums::Color::Blue);
            diag_win.end();
            diag_win.show();
            diag_win.set_callback(|_| {});

            let (rx, _handle) = downloader::download_default_mmdb();

            let mut btn = btn.clone();
            let row = row.clone();
            let mut input_bar = input_bar.clone();
            app::add_idle3(move |handle| match rx.try_recv() {
                Ok(event) => match event {
                    downloader::DownloadEvent::Progress(p) => {
                        if progress.value() as u64 != (p * 100.0) as u64 {
                            progress.set_value(p * 100.0);
                            progress.set_label(&format!("{:.0}%", p * 100.0));
                        }
                    }
                    downloader::DownloadEvent::Done(path_buf) => {
                        diag_win.hide();
                        btn.hide();
                        row.recalc();
                        input_bar.set_value(path_buf.to_str().unwrap());
                        app::remove_idle3(handle);
                    }
                },
                Err(TryRecvError::Disconnected) => {
                    diag_win.hide();
                    app::remove_idle3(handle);
                }
                Err(TryRecvError::Empty) => {}
            });
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
                buffer.set_text("Ready");
            } else {
                buffer.set_text(&format!("File does not exist '{value}'"));
            }
        });

        if downloader::default_mmdb_exists() {
            download_btn.hide();
            input_bar2.set_value(downloader::default_mmdb_path().to_str().unwrap());
            input_bar2.do_callback();
        }
    }

    let mut display = text::TextDisplay::default();
    display.set_buffer(buffer.clone());

    col.end();
    wind.end();
    wind.show();

    while app.wait() {
        if let Some(msg) = r.recv() {
            match msg {
                Message::SendQuery(msg) => {
                    if let Some(mmdb) = mmdb.borrow_mut().as_mut() {
                        let Ok(ip) = msg.parse::<IpAddr>() else {
                            buffer.set_text(&format!("Invalid IP address format '{msg}'"));
                            continue;
                        };
                        match mmdb.query_ip(ip) {
                            Ok(res) => match res {
                                Some(res) => {
                                    buffer.set_text(&format!("{res}"));
                                    display.set_insert_position(buffer.length());
                                    display.show_insert_position();
                                }
                                None => {
                                    buffer.set_text(&format!("No data found for IP '{msg}'"));
                                    continue;
                                }
                            },
                            Err(err) => {
                                buffer.set_text(&format!("Error during query '{err:?}'"));
                                continue;
                            }
                        }
                    } else {
                        buffer.set_text("Failed to query IP as database is not loaded");
                    }
                }
            }
        }
    }
}
