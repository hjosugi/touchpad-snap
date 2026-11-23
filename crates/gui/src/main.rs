//! `touchpad-snap-settings`: the settings window.
//!
//! It edits the same `config.json` the daemon watches, so a save applies
//! within the daemon's next poll (about two seconds) without IPC. The model,
//! validation and schema come from `touchpad-snap-core`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use eframe::egui;
use egui::{DragValue, Ui};
use touchpad_snap_core::config::{
    self, Action, Binding, Config, Direction, ExecCommand, MouseButton, PinchDirection,
    ScrollDirection, TileTarget, Trigger,
};
use touchpad_snap_core::cycle::{Order, WorkspaceScope};
use touchpad_snap_core::hotkey::Modifier;

fn main() -> eframe::Result<()> {
    let path = std::env::args_os()
        .position(|arg| arg == "--config")
        .and_then(|i| std::env::args_os().nth(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(config::default_path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 700.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "touchpad-snap settings",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(path)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Ja,
}

struct App {
    path: PathBuf,
    cfg: Config,
    problems: Vec<String>,
    selected: usize,
    lang: Lang,
    message: String,
    status: String,
    when_app: String,
    when_not_app: String,
    dirty_since: Option<usize>,
    events: Arc<Mutex<std::collections::VecDeque<String>>>,
    events_child: Option<Child>,
    export_path: String,
    import_path: String,
    pending_import: Option<Vec<Binding>>,
    undo: Option<Config>,
    saved: Config,
    capture_hotkey: bool,
    available_apps: Vec<String>,
    writable: bool,
}

impl App {
    fn new(path: PathBuf) -> Self {
        let (cfg, problems) = load(&path);
        let load_error = problems
            .first()
            .filter(|p| p.contains("could not load"))
            .cloned();
        let legacy = problems.iter().any(|p| p.contains("legacy config.toml"));
        let mut app = Self {
            path,
            cfg: cfg.clone(),
            problems,
            selected: 0,
            lang: Lang::En,
            message: String::new(),
            status: String::new(),
            when_app: String::new(),
            when_not_app: String::new(),
            dirty_since: None,
            events: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            events_child: None,
            export_path: std::env::temp_dir()
                .join("touchpad-snap-bindings.json")
                .display()
                .to_string(),
            import_path: String::new(),
            pending_import: None,
            undo: None,
            saved: cfg.clone(),
            capture_hotkey: false,
            available_apps: Vec::new(),
            writable: load_error.is_none() && !legacy,
        };
        app.sync_when();
        app
    }

    fn start_events(&mut self) {
        if self.events_child.is_some() {
            return;
        }
        self.events
            .lock()
            .expect("events")
            .push_back(format!("— {} —", self.t("listening", "待ち受け中")));
        match Command::new("touchpad-snap")
            .arg("events")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    let buffer = self.events.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                            let mut buffer = buffer.lock().expect("events");
                            buffer.push_back(line);
                            while buffer.len() > 300 {
                                buffer.pop_front();
                            }
                        }
                    });
                }
                self.events_child = Some(child);
            }
            Err(e) => {
                self.message = format!("{} {e}", self.t("events:", "events:"));
            }
        }
    }

    fn stop_events(&mut self) {
        if let Some(mut child) = self.events_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn try_selected(&mut self) {
        let Some(binding) = self.cfg.bindings.get(self.selected) else {
            return;
        };
        let mut command = Command::new("touchpad-snap");
        command.arg("run");
        match &binding.name {
            Some(name) if !name.trim().is_empty() => {
                command.arg("--binding").arg(name);
            }
            _ => match binding.actions.first() {
                Some(action) => match serde_json::to_string(action) {
                    Ok(json) => {
                        command.arg(json);
                    }
                    Err(e) => {
                        self.message = format!("{e}");
                        return;
                    }
                },
                None => {
                    self.message = self
                        .t("Add an action first.", "先にアクションを追加してください。")
                        .into();
                    return;
                }
            },
        }
        self.message = match command.output() {
            Ok(output) if output.status.success() => {
                format!(
                    "{} {}",
                    self.t("Ran", "実行しました:"),
                    String::from_utf8_lossy(&output.stdout).trim()
                )
            }
            Ok(output) => format!(
                "{} {}",
                self.t("Error:", "エラー:"),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(e) => format!("{e}"),
        };
    }

    fn export_bindings(&mut self) {
        let path = PathBuf::from(self.export_path.trim());
        let text = match serde_json::to_string_pretty(&self.cfg.bindings) {
            Ok(text) => text + "\n",
            Err(e) => {
                self.message = format!("{e}");
                return;
            }
        };
        self.message = match std::fs::write(&path, text) {
            Ok(()) => format!(
                "{} {}",
                self.t("Exported to", "書き出しました:"),
                path.display()
            ),
            Err(e) => format!("{} {e}", self.t("Error:", "エラー:")),
        };
    }

    fn prepare_import(&mut self) {
        let path = PathBuf::from(self.import_path.trim());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                self.message = format!("{} {e}", self.t("Error:", "エラー:"));
                return;
            }
        };
        let imported: Vec<Binding> = match serde_json::from_str::<Vec<Binding>>(&text) {
            Ok(bindings) => bindings,
            Err(_) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => match value.get("bindings").cloned() {
                    Some(bindings) => match serde_json::from_value::<Vec<Binding>>(bindings) {
                        Ok(bindings) => bindings,
                        Err(e) => {
                            self.message = format!("{} {e}", self.t("Error:", "エラー:"));
                            return;
                        }
                    },
                    None => {
                        self.message = self
                            .t(
                                "No bindings in that file.",
                                "そのファイルにバインディングがありません。",
                            )
                            .into();
                        return;
                    }
                },
                Err(e) => {
                    self.message = format!("{} {e}", self.t("Error:", "エラー:"));
                    return;
                }
            },
        };
        if imported.is_empty() {
            self.message = self
                .t(
                    "No bindings in that file.",
                    "そのファイルにバインディングがありません。",
                )
                .into();
            return;
        }
        self.pending_import = Some(imported);
    }

    fn apply_import(&mut self) {
        let Some(imported) = self.pending_import.take() else {
            return;
        };
        self.stop_events();
        let count = imported.len();
        self.cfg.bindings.extend(imported);
        self.selected = self.cfg.bindings.len() - 1;
        self.sync_when();
        self.message = format!(
            "{} {count} {}",
            self.t("Imported", "取り込みました:"),
            self.t("binding(s); save to apply", "件。保存で反映されます")
        );
    }

    fn t(&self, en: &'static str, ja: &'static str) -> &'static str {
        match self.lang {
            Lang::En => en,
            Lang::Ja => ja,
        }
    }

    fn sync_when(&mut self) {
        let binding = self.cfg.bindings.get(self.selected);
        self.when_app = binding
            .and_then(|b| b.when.as_ref())
            .map(|w| w.app.join(", "))
            .unwrap_or_default();
        self.when_not_app = binding
            .and_then(|b| b.when.as_ref())
            .map(|w| w.not_app.join(", "))
            .unwrap_or_default();
        self.dirty_since = Some(self.selected);
    }

    fn reload(&mut self) {
        let (cfg, problems) = load(&self.path);
        self.writable = !problems
            .first()
            .is_some_and(|p| p.contains("could not load"));
        self.cfg = cfg.clone();
        self.saved = cfg;
        self.problems = problems;
        self.undo = None;
        self.selected = self.selected.min(self.cfg.bindings.len().saturating_sub(1));
        self.sync_when();
        self.message = self.t("Reloaded", "再読み込みしました").into();
    }

    fn save(&mut self) {
        if !self.writable {
            self.message = self
                .t(
                    "The file could not be parsed; fix it before saving.",
                    "ファイルを解析できないため保存できません。修正してください。",
                )
                .into();
            return;
        }
        if let Some(binding) = self.cfg.bindings.get_mut(self.selected) {
            apply_when(binding, &self.when_app, &self.when_not_app);
        }
        for (index, binding) in self.cfg.bindings.iter_mut().enumerate() {
            binding.index = index;
        }
        match write_atomic(&self.path, &self.cfg) {
            Ok(()) => {
                self.undo = Some(self.saved.clone());
                self.saved = self.cfg.clone();
                self.message = format!(
                    "{} {}",
                    self.t("Saved", "保存しました"),
                    self.path.display()
                );
                let (cfg, problems) = load(&self.path);
                self.cfg = cfg.clone();
                self.saved = cfg;
                self.problems = problems;
            }
            Err(e) => self.message = format!("{} {e:#}", self.t("Error:", "エラー:")),
        }
    }

    fn refresh_status(&mut self) {
        let output = Command::new("touchpad-snap")
            .args(["status", "--json"])
            .output();
        self.status = match output {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(value) => {
                        let failed = value["hotkeys_failed"]
                            .as_array()
                            .map(|list| {
                                list.iter()
                                    .filter_map(|key| key.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let suffix = if failed.is_empty() {
                            String::new()
                        } else {
                            format!(" · {} {failed}", self.t("taken:", "取得できず:"))
                        };
                        format!(
                            "{} {} · {} · {} {}{suffix}",
                            self.t("daemon", "daemon"),
                            value["version"].as_str().unwrap_or("?"),
                            value["backend"].as_str().unwrap_or("?"),
                            value["bindings"].as_array().map_or(0, Vec::len),
                            self.t("bindings", "バインディング")
                        )
                    }
                    Err(_) => text.trim().to_string(),
                }
            }
            Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_string(),
            Err(_) => self
                .t("daemon not running", "daemon は動いていません")
                .into(),
        };
    }

    fn add_binding(&mut self) {
        let mut binding = Binding {
            name: None,
            enabled: true,
            trigger: Trigger::Hotkey {
                keys: "Ctrl+Alt+N".into(),
            },
            when: None,
            repeat: None,
            actions: vec![Action::Next],
            index: self.cfg.bindings.len(),
        };
        binding.name = Some(match self.lang {
            Lang::En => format!("Binding {}", self.cfg.bindings.len() + 1),
            Lang::Ja => format!("バインディング {}", self.cfg.bindings.len() + 1),
        });
        self.cfg.bindings.push(binding);
        self.selected = self.cfg.bindings.len() - 1;
        self.sync_when();
    }

    fn duplicate(&mut self) {
        let Some(binding) = self.cfg.bindings.get(self.selected) else {
            return;
        };
        let mut copy = binding.clone();
        copy.name = Some(format!(
            "{} ({})",
            copy.name
                .unwrap_or_else(|| self.t("Binding", "バインディング").into()),
            self.t("copy", "コピー")
        ));
        self.cfg.bindings.insert(self.selected + 1, copy);
        self.selected += 1;
        self.sync_when();
    }

    fn delete(&mut self) {
        if self.selected < self.cfg.bindings.len() {
            self.cfg.bindings.remove(self.selected);
        }
        self.selected = self.selected.min(self.cfg.bindings.len().saturating_sub(1));
        self.sync_when();
    }

    fn move_by(&mut self, by: isize) {
        let len = self.cfg.bindings.len();
        if len < 2 {
            return;
        }
        let target = self.selected as isize + by;
        if target < 0 || target >= len as isize {
            return;
        }
        self.cfg.bindings.swap(self.selected, target as usize);
        self.selected = target as usize;
        self.sync_when();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop_events();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.capture_hotkey {
            if let Some((key, mods)) = ctx.input(capture_key) {
                if key == egui::Key::Escape {
                    self.capture_hotkey = false;
                } else {
                    let mut parts = Vec::new();
                    if mods.shift {
                        parts.push("Shift".to_string());
                    }
                    if mods.ctrl {
                        parts.push("Ctrl".to_string());
                    }
                    if mods.alt {
                        parts.push("Alt".to_string());
                    }
                    if let Some(binding) = self.cfg.bindings.get_mut(self.selected) {
                        let mut keys = parts.join("+");
                        if !keys.is_empty() {
                            keys.push('+');
                        }
                        keys.push_str(&hotkey_key_name(key));
                        binding.trigger = Trigger::Hotkey { keys };
                    }
                    self.capture_hotkey = false;
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if let Some(child) = &mut self.events_child
            && matches!(child.try_wait(), Ok(Some(_)))
        {
            self.events_child = None;
        }
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("touchpad-snap");
                ui.separator();
                if ui
                    .add_enabled(self.writable, egui::Button::new(self.t("Save", "保存")))
                    .clicked()
                {
                    self.save();
                }
                if ui.button(self.t("Reload", "再読み込み")).clicked() {
                    self.reload();
                }
                if ui
                    .add_enabled(
                        self.undo.is_some(),
                        egui::Button::new(self.t("Undo", "元に戻す")),
                    )
                    .clicked()
                    && let Some(undo) = self.undo.take()
                {
                    self.cfg = undo;
                    self.sync_when();
                    self.message = self
                        .t("Undid the last save", "直前の保存を元に戻しました")
                        .into();
                }
                if ui.button(self.t("Check daemon", "daemon を確認")).clicked() {
                    self.refresh_status();
                }
                if ui.button(self.t("Try", "試す")).clicked() {
                    self.try_selected();
                }
                if self.events_child.is_some() {
                    if ui.button(self.t("Stop live", "ライブ表示を停止")).clicked() {
                        self.stop_events();
                    }
                } else if ui.button(self.t("Live", "ライブ表示")).clicked() {
                    self.start_events();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(self.lang == Lang::Ja, "日本語")
                        .clicked()
                    {
                        self.lang = Lang::Ja;
                    }
                    if ui
                        .selectable_label(self.lang == Lang::En, "English")
                        .clicked()
                    {
                        self.lang = Lang::En;
                    }
                });
            });
            ui.label(&self.message);
            if !self.status.is_empty() {
                ui.label(egui::RichText::new(&self.status).weak());
            }
        });

        egui::SidePanel::left("bindings")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(self.t("+ Add", "+ 追加")).clicked() {
                        self.add_binding();
                    }
                    if ui.button(self.t("Copy", "複製")).clicked() {
                        self.duplicate();
                    }
                    if ui.button(self.t("Delete", "削除")).clicked() {
                        self.delete();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("↑").clicked() {
                        self.move_by(-1);
                    }
                    if ui.button("↓").clicked() {
                        self.move_by(1);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut select = None;
                    for index in 0..self.cfg.bindings.len() {
                        let binding = &mut self.cfg.bindings[index];
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut binding.enabled, "");
                            let label = format!(
                                "{} → {}",
                                trigger_summary(&binding.trigger),
                                actions_summary(&binding.actions)
                            );
                            let name = binding.name.clone().unwrap_or(label.clone());
                            if ui.selectable_label(index == self.selected, name).clicked() {
                                select = Some(index);
                            }
                        });
                        ui.label(
                            egui::RichText::new(format!(
                                "   {} → {}",
                                trigger_summary(&binding.trigger),
                                actions_summary(&binding.actions)
                            ))
                            .small()
                            .weak(),
                        );
                    }
                    if let Some(index) = select {
                        self.selected = index;
                        self.sync_when();
                    }
                });
            });

        if self.events_child.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::TopBottomPanel::bottom("files")
            .resizable(true)
            .default_height(180.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(self.t("Export bindings", "バインディングを書き出す"));
                    ui.text_edit_singleline(&mut self.export_path);
                    if ui.button("Export").clicked() {
                        self.export_bindings();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Import bindings", "バインディングを取り込む"));
                    ui.text_edit_singleline(&mut self.import_path);
                    if ui.button("Import").clicked() {
                        self.prepare_import();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(self.t("Live events", "ライブイベント"));
                    if ui.button(self.t("Clear", "消去")).clicked() {
                        self.events.lock().expect("events").clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let lines: Vec<String> = self
                            .events
                            .lock()
                            .expect("events")
                            .iter()
                            .cloned()
                            .collect();
                        for line in lines {
                            ui.label(egui::RichText::new(line).monospace().small());
                        }
                    });
            });

        if let Some(imported) = self.pending_import.clone() {
            let execs: Vec<String> = imported
                .iter()
                .flat_map(|binding| binding.actions.iter())
                .filter_map(|action| match action {
                    Action::Exec { command, .. } => Some(command.display()),
                    _ => None,
                })
                .collect();
            egui::Window::new(self.t("Import bindings?", "バインディングを取り込みますか？"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{} {}",
                        imported.len(),
                        self.t(
                            "binding(s) will be added.",
                            "件のバインディングを追加します。"
                        )
                    ));
                    if !execs.is_empty() {
                        ui.label(
                            egui::RichText::new(self.t(
                                "These commands are run by exec actions:",
                                "これらのコマンドが exec アクションで実行されます:",
                            ))
                            .strong(),
                        );
                        for command in &execs {
                            ui.label(egui::RichText::new(command).monospace());
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button(self.t("Add them", "追加する")).clicked() {
                            self.apply_import();
                        }
                        if ui.button(self.t("Cancel", "やめる")).clicked() {
                            self.pending_import = None;
                        }
                    });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.binding_editor(ui);
                ui.add_space(12.0);
                self.general_settings(ui);
                ui.add_space(12.0);
                self.problems_panel(ui);
            });
        });
    }
}

impl App {
    fn binding_editor(&mut self, ui: &mut Ui) {
        ui.heading(self.t("Binding", "バインディング"));
        let Some(mut binding) = self.cfg.bindings.get(self.selected).cloned() else {
            ui.label(self.t("No binding selected.", "バインディングがありません。"));
            return;
        };

        ui.horizontal(|ui| {
            ui.label(self.t("Name", "名前"));
            let mut name = binding.name.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut name).changed() {
                binding.name = if name.trim().is_empty() {
                    None
                } else {
                    Some(name)
                };
            }
            ui.checkbox(&mut binding.enabled, self.t("Enabled", "有効"));
        });

        ui.horizontal(|ui| {
            ui.label(self.t("Repeat", "繰り返し"));
            let mut repeat = binding.repeat;
            ui.selectable_value(&mut repeat, None, self.t("Automatic", "自動"));
            ui.selectable_value(&mut repeat, Some(true), self.t("Yes", "する"));
            ui.selectable_value(&mut repeat, Some(false), self.t("No", "しない"));
            binding.repeat = repeat;
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(self.t("Trigger", "トリガー"));
            if ui
                .button(self.t("Capture shortcut", "ショートカットを記録"))
                .clicked()
            {
                self.capture_hotkey = true;
            }
            if self.capture_hotkey {
                ui.label(self.t(
                    "Press the keys… (Esc cancels)",
                    "キーを押してください…（Esc で中止）",
                ));
            }
        });
        trigger_editor(ui, &mut binding.trigger, self.lang);

        ui.separator();
        ui.label(self.t("When (optional)", "条件（任意）"));
        ui.horizontal(|ui| {
            ui.label(self.t("Apps", "アプリ"));
            ui.text_edit_singleline(&mut self.when_app);
        });
        ui.horizontal(|ui| {
            ui.label(self.t("Not apps", "除外アプリ"));
            ui.text_edit_singleline(&mut self.when_not_app);
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.t("Open apps", "開いているアプリ")).clicked() {
                self.available_apps = open_apps();
            }
            let mut chosen = None;
            for app in &self.available_apps {
                if ui.small_button(app).clicked() {
                    chosen = Some(app.clone());
                }
            }
            if let Some(app) = chosen
                && !self.when_app.split(',').any(|part| part.trim() == app)
            {
                if !self.when_app.trim().is_empty() {
                    self.when_app.push_str(", ");
                }
                self.when_app.push_str(&app);
            }
        });
        apply_when(&mut binding, &self.when_app, &self.when_not_app);

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(self.t("Actions", "アクション"));
            if ui.button("+").clicked() {
                binding.actions.push(Action::Next);
            }
        });
        let mut remove = None;
        for index in 0..binding.actions.len() {
            ui.horizontal(|ui| {
                action_editor(ui, &mut binding.actions[index], self.lang);
                if ui.button("×").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            binding.actions.remove(index);
        }
        if binding.actions.is_empty() {
            ui.label(
                egui::RichText::new(self.t(
                    "A binding needs at least one action.",
                    "アクションが 1 つ以上必要です。",
                ))
                .color(egui::Color32::LIGHT_RED),
            );
        }
        self.cfg.bindings[self.selected] = binding;
    }

    fn general_settings(&mut self, ui: &mut Ui) {
        ui.heading(self.t("General", "全般"));
        ui.horizontal(|ui| {
            ui.label(self.t("Order", "順序"));
            egui::ComboBox::from_id_salt("order")
                .selected_text(format!("{:?}", self.cfg.cycle.order))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.cycle.order, Order::Stable, "stable");
                    ui.selectable_value(&mut self.cfg.cycle.order, Order::Position, "position");
                    ui.selectable_value(&mut self.cfg.cycle.order, Order::Mru, "mru");
                });
            ui.separator();
            ui.label(self.t("Workspace", "ワークスペース"));
            egui::ComboBox::from_id_salt("workspace")
                .selected_text(format!("{:?}", self.cfg.cycle.workspace))
                .show_ui(ui, |ui| {
                    let all = self.t("all", "すべて");
                    let current = self.t("current", "現在");
                    ui.selectable_value(&mut self.cfg.cycle.workspace, WorkspaceScope::All, all);
                    ui.selectable_value(
                        &mut self.cfg.cycle.workspace,
                        WorkspaceScope::Current,
                        current,
                    );
                });
        });
        ui.horizontal(|ui| {
            let include = self.t("Include minimized windows", "最小化したウィンドウを含める");
            ui.checkbox(&mut self.cfg.cycle.include_minimized, include);
            ui.separator();
            ui.label(self.t("MRU session (ms)", "MRU セッション (ms)"));
            ui.add(DragValue::new(&mut self.cfg.cycle.mru_session_ms).range(100..=10_000));
        });
        ui.horizontal(|ui| {
            ui.label(self.t("Swipe threshold", "スワイプしきい値"));
            ui.add(DragValue::new(&mut self.cfg.gesture.threshold).range(10.0..=1000.0));
            ui.label(self.t("Repeat step", "繰り返し幅"));
            ui.add(DragValue::new(&mut self.cfg.gesture.repeat_step).range(0.0..=1000.0));
        });
        ui.horizontal(|ui| {
            let overlay = self.t("Overlay", "オーバーレイ");
            ui.checkbox(&mut self.cfg.hud.enabled, overlay);
            ui.label(self.t("Duration (ms)", "表示時間 (ms)"));
            ui.add(DragValue::new(&mut self.cfg.hud.duration_ms).range(100..=10_000));
        });
        ui.label(
            egui::RichText::new(format!(
                "{}: {}",
                self.t("Config file", "設定ファイル"),
                self.path.display()
            ))
            .weak(),
        );
    }

    fn problems_panel(&mut self, ui: &mut Ui) {
        if self.problems.is_empty() {
            return;
        }
        ui.heading(self.t("Problems", "問題"));
        for problem in &self.problems {
            ui.label(egui::RichText::new(problem).color(egui::Color32::LIGHT_RED));
        }
    }
}

fn load(path: &Path) -> (Config, Vec<String>) {
    match Config::load_report(path) {
        Ok(cfg) => {
            let problems = cfg.report().problems.iter().map(|p| p.describe()).collect();
            (cfg, problems)
        }
        Err(e) => (
            Config::default(),
            vec![format!("could not load {}: {e:#}", path.display())],
        ),
    }
}

fn write_atomic(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, config::to_json(cfg))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn apply_when(binding: &mut Binding, app_text: &str, not_app_text: &str) {
    let app = split_list(app_text);
    let not_app = split_list(not_app_text);
    match (&mut binding.when, app.is_empty() && not_app.is_empty()) {
        (Some(when), false) => {
            when.app = app;
            when.not_app = not_app;
        }
        (Some(when), true) if when.os.is_empty() && when.desktop.is_empty() => {
            binding.when = None;
        }
        (Some(when), true) => {
            when.app.clear();
            when.not_app.clear();
        }
        (None, false) => {
            binding.when = Some(touchpad_snap_core::when::When {
                app,
                not_app,
                ..Default::default()
            });
        }
        (None, true) => {}
    }
}

fn modifiers_ui(ui: &mut Ui, modifiers: &mut Vec<Modifier>) {
    for (modifier, label) in [
        (Modifier::Ctrl, "Ctrl"),
        (Modifier::Alt, "Alt"),
        (Modifier::Shift, "Shift"),
        (Modifier::Super, "Super"),
    ] {
        let mut on = modifiers.contains(&modifier);
        if ui.checkbox(&mut on, label).changed() {
            if on {
                modifiers.push(modifier);
            } else {
                modifiers.retain(|m| *m != modifier);
            }
        }
    }
}

fn trigger_summary(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Hotkey { keys } => keys.clone(),
        Trigger::Swipe {
            fingers,
            modifiers,
            direction,
        } => format!(
            "{}{} fingers {:?}",
            mods_prefix(modifiers),
            fingers,
            direction
        ),
        Trigger::Pinch {
            fingers,
            modifiers,
            direction,
        } => format!(
            "{}{} finger pinch {}",
            mods_prefix(modifiers),
            fingers,
            direction.as_str()
        ),
        Trigger::Hold { fingers, modifiers } => {
            format!("{}{} finger hold", mods_prefix(modifiers), fingers)
        }
        Trigger::DoubleTap { modifier } => format!("{:?} double-tap", modifier),
        Trigger::Sequence { keys, then } => format!("{keys}, then {then}"),
        Trigger::Mouse {
            button,
            modifiers,
            scroll,
        } => match scroll {
            Some(scroll) => format!(
                "{}mouse {} + scroll {}",
                mods_prefix(modifiers),
                button.as_str(),
                scroll.as_str()
            ),
            None => format!("{}mouse {}", mods_prefix(modifiers), button.as_str()),
        },
    }
}

fn mods_prefix(modifiers: &[Modifier]) -> String {
    modifiers
        .iter()
        .map(|m| format!("{m:?}+"))
        .collect::<String>()
}

fn actions_summary(actions: &[Action]) -> String {
    actions
        .iter()
        .map(|a| a.label())
        .collect::<Vec<_>>()
        .join(" + ")
}

fn trigger_editor(ui: &mut Ui, trigger: &mut Trigger, lang: Lang) {
    let kinds = [
        ("hotkey", lang),
        ("swipe", lang),
        ("pinch", lang),
        ("hold", lang),
        ("double_tap", lang),
        ("sequence", lang),
        ("mouse", lang),
    ];
    ui.horizontal_wrapped(|ui| {
        for (kind, _) in kinds {
            let selected =
                std::mem::discriminant(trigger) == std::mem::discriminant(&default_trigger(kind));
            if ui.selectable_label(selected, kind).clicked() && !selected {
                *trigger = default_trigger(kind);
            }
        }
    });
    match trigger {
        Trigger::Hotkey { keys } => {
            ui.horizontal(|ui| {
                ui.label("Keys");
                ui.text_edit_singleline(keys);
            });
        }
        Trigger::Swipe {
            fingers,
            modifiers,
            direction,
        } => {
            ui.horizontal(|ui| {
                ui.label("Fingers");
                ui.add(DragValue::new(fingers).range(3..=5));
                direction_ui(ui, direction);
            });
            ui.horizontal(|ui| modifiers_ui(ui, modifiers));
        }
        Trigger::Pinch {
            fingers,
            modifiers,
            direction,
        } => {
            ui.horizontal(|ui| {
                ui.label("Fingers");
                ui.add(DragValue::new(fingers).range(2..=5));
                ui.selectable_value(direction, PinchDirection::In, "in");
                ui.selectable_value(direction, PinchDirection::Out, "out");
            });
            ui.horizontal(|ui| modifiers_ui(ui, modifiers));
        }
        Trigger::Hold { fingers, modifiers } => {
            ui.horizontal(|ui| {
                ui.label("Fingers");
                ui.add(DragValue::new(fingers).range(2..=5));
            });
            ui.horizontal(|ui| modifiers_ui(ui, modifiers));
        }
        Trigger::DoubleTap { modifier } => {
            modifier_ui(ui, modifier);
        }
        Trigger::Sequence { keys, then } => {
            ui.horizontal(|ui| {
                ui.label("Prefix");
                ui.text_edit_singleline(keys);
                ui.label("Then");
                ui.text_edit_singleline(then);
            });
        }
        Trigger::Mouse {
            button,
            modifiers,
            scroll,
        } => {
            ui.horizontal(|ui| {
                ui.label("Button");
                for (value, label) in [
                    (MouseButton::Left, "left"),
                    (MouseButton::Right, "right"),
                    (MouseButton::Middle, "middle"),
                    (MouseButton::Back, "back"),
                    (MouseButton::Forward, "forward"),
                ] {
                    ui.selectable_value(button, value, label);
                }
            });
            ui.horizontal(|ui| {
                let mut on = scroll.is_some();
                if ui.checkbox(&mut on, "Scroll").changed() {
                    *scroll = on.then_some(ScrollDirection::Up);
                }
                if let Some(scroll) = scroll {
                    for (value, label) in [
                        (ScrollDirection::Up, "up"),
                        (ScrollDirection::Down, "down"),
                        (ScrollDirection::Left, "left"),
                        (ScrollDirection::Right, "right"),
                    ] {
                        ui.selectable_value(scroll, value, label);
                    }
                }
            });
            ui.horizontal(|ui| modifiers_ui(ui, modifiers));
        }
    }
}

fn modifier_ui(ui: &mut Ui, modifier: &mut Modifier) {
    ui.horizontal(|ui| {
        ui.label("Modifier");
        for (value, label) in [
            (Modifier::Ctrl, "Ctrl"),
            (Modifier::Alt, "Alt"),
            (Modifier::Shift, "Shift"),
            (Modifier::Super, "Super"),
        ] {
            ui.selectable_value(modifier, value, label);
        }
    });
}

fn direction_ui(ui: &mut Ui, direction: &mut touchpad_snap_core::gesture::SwipeDirection) {
    use touchpad_snap_core::gesture::SwipeDirection;
    for (value, label) in [
        (SwipeDirection::Left, "left"),
        (SwipeDirection::Right, "right"),
        (SwipeDirection::Up, "up"),
        (SwipeDirection::Down, "down"),
    ] {
        ui.selectable_value(direction, value, label);
    }
}

fn default_trigger(kind: &str) -> Trigger {
    match kind {
        "swipe" => Trigger::Swipe {
            fingers: 3,
            modifiers: vec![Modifier::Ctrl],
            direction: touchpad_snap_core::gesture::SwipeDirection::Left,
        },
        "pinch" => Trigger::Pinch {
            fingers: 2,
            modifiers: Vec::new(),
            direction: PinchDirection::In,
        },
        "hold" => Trigger::Hold {
            fingers: 2,
            modifiers: Vec::new(),
        },
        "double_tap" => Trigger::DoubleTap {
            modifier: Modifier::Ctrl,
        },
        "sequence" => Trigger::Sequence {
            keys: "Super+Alt+W".into(),
            then: "M".into(),
        },
        "mouse" => Trigger::Mouse {
            button: MouseButton::Back,
            modifiers: Vec::new(),
            scroll: None,
        },
        _ => Trigger::Hotkey {
            keys: "Ctrl+Alt+N".into(),
        },
    }
}

fn action_label(kind: &str) -> &'static str {
    match kind {
        "window.prev" => "window.prev",
        "app.next" => "app.next",
        "app.prev" => "app.prev",
        "delay" => "delay",
        "exec" => "exec",
        "keys" => "keys",
        "window.minimize" => "window.minimize",
        "window.maximize" => "window.maximize",
        "window.restore" => "window.restore",
        "window.toggle_maximize" => "window.toggle_maximize",
        "window.fullscreen" => "window.fullscreen",
        "window.close" => "window.close",
        "window.tile" => "window.tile",
        "window.move_to_monitor" => "window.move_to_monitor",
        "window.move_to_workspace" => "window.move_to_workspace",
        "workspace.next" => "workspace.next",
        "workspace.prev" => "workspace.prev",
        "overview.toggle" => "overview.toggle",
        _ => "window.next",
    }
}

const ACTION_KINDS: &[&str] = &[
    "window.next",
    "window.prev",
    "app.next",
    "app.prev",
    "window.minimize",
    "window.maximize",
    "window.restore",
    "window.toggle_maximize",
    "window.fullscreen",
    "window.close",
    "window.tile",
    "window.move_to_monitor",
    "window.move_to_workspace",
    "workspace.next",
    "workspace.prev",
    "overview.toggle",
    "keys",
    "exec",
    "delay",
];

fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::Next => "window.next",
        Action::Prev => "window.prev",
        Action::AppNext => "app.next",
        Action::AppPrev => "app.prev",
        Action::Delay { .. } => "delay",
        Action::Exec { .. } => "exec",
        Action::Keys { .. } => "keys",
        Action::WindowMinimize => "window.minimize",
        Action::WindowMaximize => "window.maximize",
        Action::WindowRestore => "window.restore",
        Action::WindowToggleMaximize => "window.toggle_maximize",
        Action::WindowFullscreen => "window.fullscreen",
        Action::WindowClose => "window.close",
        Action::WindowTile { .. } => "window.tile",
        Action::WindowMoveToMonitor { .. } => "window.move_to_monitor",
        Action::WindowMoveToWorkspace { .. } => "window.move_to_workspace",
        Action::WorkspaceNext => "workspace.next",
        Action::WorkspacePrev => "workspace.prev",
        Action::OverviewToggle => "overview.toggle",
    }
}

fn default_action(kind: &str) -> Action {
    match kind {
        "window.prev" => Action::Prev,
        "app.next" => Action::AppNext,
        "app.prev" => Action::AppPrev,
        "delay" => Action::Delay { ms: 100 },
        "exec" => Action::Exec {
            command: ExecCommand::Shell("notify-send touchpad-snap".into()),
            shell: true,
        },
        "keys" => Action::Keys {
            keys: "Ctrl+W".into(),
        },
        "window.minimize" => Action::WindowMinimize,
        "window.maximize" => Action::WindowMaximize,
        "window.restore" => Action::WindowRestore,
        "window.toggle_maximize" => Action::WindowToggleMaximize,
        "window.fullscreen" => Action::WindowFullscreen,
        "window.close" => Action::WindowClose,
        "window.tile" => Action::WindowTile {
            to: TileTarget::Left,
        },
        "window.move_to_monitor" => Action::WindowMoveToMonitor {
            to: Direction::Next,
        },
        "window.move_to_workspace" => Action::WindowMoveToWorkspace {
            to: Direction::Next,
        },
        "workspace.next" => Action::WorkspaceNext,
        "workspace.prev" => Action::WorkspacePrev,
        "overview.toggle" => Action::OverviewToggle,
        _ => Action::Next,
    }
}

fn action_editor(ui: &mut Ui, action: &mut Action, _lang: Lang) {
    let current = action_kind(action);
    egui::ComboBox::from_id_salt(("action", action as *mut Action as usize))
        .selected_text(current)
        .show_ui(ui, |ui| {
            for kind in ACTION_KINDS {
                if ui
                    .selectable_label(*kind == current, action_label(kind))
                    .clicked()
                    && *kind != current
                {
                    *action = default_action(kind);
                }
            }
        });
    match action {
        Action::Delay { ms } => {
            ui.add(DragValue::new(ms).range(0..=60_000));
            ui.label("ms");
        }
        Action::Keys { keys } => {
            ui.label("keys");
            ui.text_edit_singleline(keys);
        }
        Action::Exec { command, shell } => {
            ui.checkbox(shell, "shell");
            if *shell {
                if !matches!(command, ExecCommand::Shell(_)) {
                    *command = ExecCommand::Shell(String::new());
                }
                if let ExecCommand::Shell(script) = command {
                    ui.text_edit_singleline(script);
                }
            } else {
                if let ExecCommand::Shell(script) = command {
                    *command = ExecCommand::Args(split_whitespace(script));
                }
                if let ExecCommand::Args(args) = command {
                    let mut text = args.join(" ");
                    if ui.text_edit_singleline(&mut text).changed() {
                        *args = split_whitespace(&text);
                    }
                }
            }
        }
        Action::WindowTile { to } => {
            for (value, label) in [
                (TileTarget::Left, "left"),
                (TileTarget::Right, "right"),
                (TileTarget::Top, "top"),
                (TileTarget::Bottom, "bottom"),
                (TileTarget::TopLeft, "top-left"),
                (TileTarget::TopRight, "top-right"),
                (TileTarget::BottomLeft, "bottom-left"),
                (TileTarget::BottomRight, "bottom-right"),
            ] {
                ui.selectable_value(to, value, label);
            }
        }
        Action::WindowMoveToMonitor { to } | Action::WindowMoveToWorkspace { to } => {
            ui.selectable_value(to, Direction::Next, "next");
            ui.selectable_value(to, Direction::Prev, "prev");
        }
        _ => {}
    }
}

fn split_whitespace(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn capture_key(input: &egui::InputState) -> Option<(egui::Key, egui::Modifiers)> {
    input.events.iter().find_map(|event| match event {
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => Some((*key, *modifiers)),
        _ => None,
    })
}

fn hotkey_key_name(key: egui::Key) -> String {
    match key {
        egui::Key::Space => "space".into(),
        egui::Key::Enter => "return".into(),
        egui::Key::Escape => "escape".into(),
        egui::Key::Tab => "tab".into(),
        egui::Key::Backspace => "backspace".into(),
        egui::Key::Delete => "delete".into(),
        egui::Key::ArrowLeft => "left".into(),
        egui::Key::ArrowRight => "right".into(),
        egui::Key::ArrowUp => "up".into(),
        egui::Key::ArrowDown => "down".into(),
        egui::Key::Home => "home".into(),
        egui::Key::End => "end".into(),
        egui::Key::PageUp => "pageup".into(),
        egui::Key::PageDown => "pagedown".into(),
        other => format!("{other:?}"),
    }
}

fn open_apps() -> Vec<String> {
    let output = Command::new("touchpad-snap")
        .args(["list", "--json"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    let mut apps: Vec<String> = Vec::new();
    if let Some(windows) = value["windows"].as_array() {
        for window in windows {
            if let Some(app) = window["app_id"].as_str().filter(|a| !a.is_empty())
                && !apps.iter().any(|known| known == app)
            {
                apps.push(app.to_string());
            }
        }
    }
    apps.sort();
    apps
}
