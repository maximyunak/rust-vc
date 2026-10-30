#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod call;

use call::{list_devices, Call};
use common::{format_link, parse_link, rand_u64};
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, Sense, Vec2};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;
use single_instance::SingleInstance;

// ── палитра ────────────────────────────────────────────────
const BG: Color32 = Color32::from_rgb(3, 5, 22);
const BG_BAR: Color32 = Color32::from_rgb(10, 12, 36);
const CARD: Color32 = Color32::from_rgb(138, 159, 255);
const TILE: Color32 = Color32::from_rgb(18, 21, 52);
const CHIP: Color32 = Color32::from_rgb(28, 32, 72);
const CHIP_HOVER: Color32 = Color32::from_rgb(40, 46, 98);
const INK: Color32 = Color32::from_rgb(15, 15, 30);
const GREEN: Color32 = Color32::from_rgb(60, 200, 90);
const RED: Color32 = Color32::from_rgb(237, 66, 69);
const RED_HOVER: Color32 = Color32::from_rgb(200, 50, 55);
const WHITE: Color32 = Color32::WHITE;

const CARD_W: f32 = 340.0;

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
    show_settings: bool,
    first_frame: bool,
}

impl App {
    fn new() -> Self {
        Self {
            name: std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "Гость".into()),
            server: "2.26.23.72:4000".into(),
            room_text: String::new(),
            inputs: list_devices(true),
            outputs: list_devices(false),
            input: None,
            output: None,
            error: None,
            link: String::new(),
            call: None,
            show_settings: false,
            first_frame: true,
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

    // ── экран входа ────────────────────────────────────────
    fn lobby(&mut self, ctx: &egui::Context) {
        let mut action: Option<bool> = None; // Some(true) = создать, Some(false) = войти

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::default().fill(BG).inner_margin(14.0))
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if pill(ui, "Настройки", 110.0, 32.0, CHIP, CHIP_HOVER, WHITE, 13.0).clicked() {
                        self.show_settings = !self.show_settings;
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG))
            .show(ctx, |ui| {
                let card_h = 270.0;
                ui.add_space(((ui.available_height() - card_h) / 2.0 - 20.0).max(0.0));
                ui.vertical_centered(|ui| {
                    egui::Frame::default()
                        .fill(CARD)
                        .rounding(14.0)
                        .inner_margin(28.0)
                        .show(ui, |ui| {
                            ui.set_width(CARD_W);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                small_label(ui, "Имя");
                                ui.add_space(4.0);
                                field(ui, &mut self.name, "Имя", CARD_W);

                                ui.add_space(16.0);
                                small_label(ui, "Подключиться по ссылке");
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;
                                    let r = field(ui, &mut self.room_text, "voice://127.0.0.1:4000/#######", CARD_W - 54.0);
                                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    if pill(ui, ">", 44.0, 40.0, WHITE, Color32::from_rgb(235, 239, 255), INK, 15.0).clicked() || enter {
                                        action = Some(false);
                                    }
                                });

                                ui.add_space(26.0);
                                if pill(ui, "Создать", CARD_W, 40.0, WHITE, Color32::from_rgb(235, 239, 255), CARD, 14.0).clicked() {
                                    action = Some(true);
                                }

                                if let Some(e) = &self.error {
                                    ui.add_space(10.0);
                                    ui.label(RichText::new(e).size(12.0).color(Color32::from_rgb(255, 225, 225)));
                                }
                            });
                        });
                });
            });

        self.settings_window(ctx);

        if let Some(create) = action {
            self.join(create);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("Настройки")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::RIGHT_TOP, [-14.0, 52.0])
            .show(ctx, |ui| {
                ui.label("Сервер (host:port)");
                ui.text_edit_singleline(&mut self.server);
                ui.add_space(8.0);
                device_combo(ui, "Микрофон", &self.inputs, &mut self.input);
                device_combo(ui, "Вывод звука", &self.outputs, &mut self.output);
            });
        self.show_settings = open;
    }

    // ── экран звонка ───────────────────────────────────────
    fn in_call(&mut self, ctx: &egui::Context) -> bool {
        let mut leave = false;
        let call = self.call.as_ref().unwrap();

        // верхняя панель: ссылка
        egui::TopBottomPanel::top("call_top")
            .frame(egui::Frame::default().fill(BG).inner_margin(16.0))
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Звонок").size(20.0).strong().color(WHITE));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill(ui, "Копировать", 110.0, 32.0, CARD, Color32::from_rgb(160, 178, 255), INK, 13.0).clicked() {
                            ctx.copy_text(self.link.clone());
                        }
                        egui::Frame::default().fill(CHIP).rounding(8.0).inner_margin(8.0).show(ui, |ui| {
                            ui.label(RichText::new(&self.link).monospace().size(12.0).color(Color32::from_rgb(190, 200, 255)));
                        });
                    });
                });
            });

        // нижняя панель: управление
        egui::TopBottomPanel::bottom("controls")
            .frame(egui::Frame::default().fill(BG_BAR).inner_margin(14.0))
            .show_separator_line(false)
            .show(ctx, |ui| {
                let widths = [130.0, 110.0, 150.0, 140.0, 90.0];
                let total: f32 = widths.iter().sum::<f32>() + 10.0 * 4.0;
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 10.0;

                    let m = call.muted.load(Relaxed);
                    let (fill, hov) = if m { (RED, RED_HOVER) } else { (CHIP, CHIP_HOVER) };
                    if pill(ui, if m { "Микрофон выкл." } else { "Микрофон" }, widths[0], 40.0, fill, hov, WHITE, 13.0).clicked() {
                        call.muted.store(!m, Relaxed);
                    }

                    let d = call.deaf.load(Relaxed);
                    let (fill, hov) = if d { (RED, RED_HOVER) } else { (CHIP, CHIP_HOVER) };
                    if pill(ui, if d { "Звук выкл." } else { "Звук" }, widths[1], 40.0, fill, hov, WHITE, 13.0).clicked() {
                        call.deaf.store(!d, Relaxed);
                    }

                    let dn = call.denoise.load(Relaxed);
                    let (fill, fg) = if dn { (CARD, INK) } else { (CHIP, WHITE) };
                    if pill(ui, "Шумоподавление", widths[2], 40.0, fill, fill, fg, 13.0).clicked() {
                        call.denoise.store(!dn, Relaxed);
                    }

                    let ec = call.echo.load(Relaxed);
                    let (fill, fg) = if ec { (CARD, INK) } else { (CHIP, WHITE) };
                    if pill(ui, "Эхоподавление", widths[3], 40.0, fill, fill, fg, 13.0).clicked() {
                        call.echo.store(!ec, Relaxed);
                    }

                    if pill(ui, "Выйти", widths[4], 40.0, RED, RED_HOVER, WHITE, 13.0).clicked() {
                        leave = true;
                    }
                });
            });

        // участники
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(14.0);
                    tile(ui, &format!("{} (вы)", self.name), &self.name, call.me_speaking());
                    for p in call.peers() {
                        tile(ui, &p.name, &p.name, p.speaking);
                    }
                });
            });

        ctx.request_repaint_after(Duration::from_millis(60));
        leave
    }
}

// ── виджеты ────────────────────────────────────────────────

fn small_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(WHITE));
}

/// Белое скруглённое поле ввода.
fn field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.scope(|ui| {
        *ui.visuals_mut() = egui::Visuals::light(); // тёмный курсор и выделение на белом
        egui::Frame::default()
            .fill(WHITE)
            .rounding(8.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(false)
                        .hint_text(hint)
                        .text_color(INK)
                        .desired_width(width - 20.0),
                )
            })
            .inner
    })
        .inner
}

/// Скруглённая кнопка.
fn pill(ui: &mut egui::Ui, label: &str, w: f32, h: f32, fill: Color32, hover: Color32, fg: Color32, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let color = if resp.hovered() { hover } else { fill };
    ui.painter().rect_filled(rect, 8.0, color);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(size), fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Плитка участника: аватар с буквой, имя, зелёная обводка при разговоре.
fn tile(ui: &mut egui::Ui, title: &str, name: &str, speaking: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 140.0), Sense::hover());
    let p = ui.painter();
    if speaking {
        p.rect_filled(rect.expand(3.0), 15.0, GREEN);
    }
    p.rect_filled(rect, 12.0, TILE);

    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(88, 101, 242),
        Color32::from_rgb(235, 69, 158),
        Color32::from_rgb(250, 166, 26),
        Color32::from_rgb(87, 160, 242),
        Color32::from_rgb(160, 90, 230),
        Color32::from_rgb(40, 180, 150),
    ];
    let h: usize = name.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());

    let c = rect.center() - Vec2::new(0.0, 14.0);
    p.circle_filled(c, 30.0, PALETTE[h % PALETTE.len()]);
    p.text(c, Align2::CENTER_CENTER, initial, FontId::proportional(26.0), WHITE);
    p.text(
        rect.center_bottom() - Vec2::new(0.0, 16.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(13.0),
        WHITE,
    );
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.first_frame {
            self.first_frame = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        if self.call.is_some() {
            if self.in_call(ctx) {
                self.call = None;
            }
        } else {
            self.lobby(ctx);
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
    let instance = SingleInstance::new("voicers_app").unwrap();

    if !instance.is_single() {
        println!("Приложение уже запущено!");
        return Ok(());
    }
    let arg = std::env::args().nth(1);
    if handle_register(&arg) {
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([700.0, 480.0])
            .with_title("VoiceRS"),
        ..Default::default()
    };
    eframe::run_native(
        "VoiceRS",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut app = App::new();
            if let Some(a) = arg.filter(|a| a.starts_with("voice://")) {
                app.room_text = a;
                app.join(false);
            }
            Ok(Box::new(app))
        }),
    )
}