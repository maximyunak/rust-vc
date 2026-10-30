#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod call;

use call::{list_devices, Call};
use common::{format_link, parse_link, rand_u64};
use eframe::egui;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

struct App {
    name: String,
    server: String,
    room_text: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
    input: Option<String>,
    output: Option<String>,
    error: Option<String>,
    link: String,
    call: Option<Call>,
}

impl App {
    fn new() -> Self {
        Self {
            name: std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "Гость".into()),
            server: "127.0.0.1:4000".into(),
            room_text: String::new(),
            inputs: list_devices(true),
            outputs: list_devices(false),
            input: None,
            output: None,
            error: None,
            link: String::new(),
            call: None,
        }
    }

    fn join(&mut self, create: bool) {
        let (server, room) = if create {
            (self.server.trim().to_string(), rand_u64())
        } else if let Some(x) = parse_link(&self.room_text) {
            x
        } else if let Ok(r) = u64::from_str_radix(self.room_text.trim(), 16) {
            (self.server.trim().to_string(), r)
        } else {
            self.error = Some("Неверная ссылка или ID комнаты".into());
            return;
        };
        match Call::start(&server, room, &self.name, self.input.as_deref(), self.output.as_deref()) {
            Ok(c) => {
                self.link = format_link(&server, room);
                self.call = Some(c);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn lobby(&mut self, ui: &mut egui::Ui) {
        ui.heading("VoiceRS");
        ui.add_space(8.0);
        ui.label("Имя");
        ui.text_edit_singleline(&mut self.name);
        ui.label("Сервер (host:port)");
        ui.text_edit_singleline(&mut self.server);

        ui.add_space(6.0);
        device_combo(ui, "Микрофон", &self.inputs, &mut self.input);
        device_combo(ui, "Вывод звука", &self.outputs, &mut self.output);

        ui.add_space(10.0);
        if ui.button("Создать звонок").clicked() {
            self.join(true);
        }
        ui.add_space(10.0);
        ui.label("Ссылка (voice://...) или ID комнаты");
        ui.text_edit_singleline(&mut self.room_text);
        if ui.button("Войти по ссылке").clicked() {
            self.join(false);
        }
        if let Some(e) = &self.error {
            ui.add_space(8.0);
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        }
    }

    fn in_call(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let mut leave = false;
        let call = self.call.as_ref().unwrap();
        ui.heading("Звонок");
        ui.label("Ссылка для друзей:");
        ui.horizontal(|ui| {
            ui.monospace(&self.link);
        });
        if ui.button("Копировать ссылку").clicked() {
            ctx.copy_text(self.link.clone());
        }
        ui.separator();

        ui.horizontal(|ui| {
            let m = call.muted.load(Relaxed);
            if ui.selectable_label(m, if m { "Микрофон выключен" } else { "Микрофон включён" }).clicked() {
                call.muted.store(!m, Relaxed);
            }
            let d = call.deaf.load(Relaxed);
            if ui.selectable_label(d, if d { "Звук выключен" } else { "Звук включён" }).clicked() {
                call.deaf.store(!d, Relaxed);
            }
        });
        ui.add_space(6.0);
        if ui.button("Выйти").clicked() {
            leave = true;
        }
        ui.separator();

        ui.label("Участники:");
        indicator(ui, &format!("{} (вы)", self.name), call.me_speaking());
        for p in call.peers() {
            indicator(ui, &p.name, p.speaking);
        }
        ctx.request_repaint_after(Duration::from_millis(60));
        leave
    }
}

fn device_combo(ui: &mut egui::Ui, label: &str, list: &[String], sel: &mut Option<String>) {
    let text = sel.clone().unwrap_or_else(|| "По умолчанию".into());
    egui::ComboBox::from_label(label).selected_text(text).width(220.0).show_ui(ui, |ui| {
        ui.selectable_value(sel, None, "По умолчанию");
        for d in list {
            ui.selectable_value(sel, Some(d.clone()), d.as_str());
        }
    });
}

fn indicator(ui: &mut egui::Ui, name: &str, speaking: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let color = if speaking { egui::Color32::from_rgb(60, 200, 90) } else { egui::Color32::DARK_GRAY };
        ui.painter().circle_filled(rect.center(), 6.0, color);
        ui.label(name);
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let mut leave = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.call.is_some() {
                leave = self.in_call(ui, ctx);
            } else {
                self.lobby(ui);
            }
        });
        if leave {
            self.call = None;
        }
    }
}

#[cfg(windows)]
fn handle_register(arg: &Option<String>) -> bool {
    if arg.as_deref() != Some("--register") {
        return false;
    }
    let exe = std::env::current_exe().unwrap();
    let cmd = format!("\"{}\" \"%1\"", exe.display());
    let reg = |args: &[&str]| {
        let _ = std::process::Command::new("reg").arg("add").args(args).status();
    };
    reg(&["HKCU\\Software\\Classes\\voice", "/ve", "/d", "URL:VoiceRS", "/f"]);
    reg(&["HKCU\\Software\\Classes\\voice", "/v", "URL Protocol", "/d", "", "/f"]);
    reg(&["HKCU\\Software\\Classes\\voice\\shell\\open\\command", "/ve", "/d", &cmd, "/f"]);
    true
}

#[cfg(not(windows))]
fn handle_register(_: &Option<String>) -> bool {
    false
}

fn main() -> eframe::Result<()> {
    let arg = std::env::args().nth(1);
    if handle_register(&arg) {
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 560.0]).with_title("VoiceRS"),
        ..Default::default()
    };
    eframe::run_native(
        "VoiceRS",
        opts,
        Box::new(move |_cc| {
            let mut app = App::new();
            if let Some(a) = arg.filter(|a| a.starts_with("voice://")) {
                app.room_text = a;
                app.join(false);
            }
            Ok(Box::new(app))
        }),
    )
}
