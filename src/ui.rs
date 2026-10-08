//! The main window, styled after getartcraft.com: compact rows divided by
//! hairlines, uppercase mono labels, square corners and the apps' own icons.

use crate::catalog::AppInfo;
use crate::icons::IconCache;
use crate::jobs::{Action, Job, Status};
use crate::manager::Manager;
use crate::settings::ThemeChoice;
use crate::theme::{self, Palette};
use crate::util::{self, human_bytes};
use crate::{logo, notify, platform, tray, APP_NAME};
use eframe::egui::{
    self, text::LayoutJob, Align, Align2, Color32, CursorIcon, FontFamily, FontId, Layout, Margin, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, TextFormat, TextStyle, Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Horizontal padding of rows and section bars.
const PAD: f32 = 14.0;

pub fn window_icon() -> egui::IconData {
    egui::IconData { rgba: logo::rgba(256, false), width: 256, height: 256 }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Apps,
    Queue,
    Settings,
}

/// What the window shows for one app, copied out of the shared state so the
/// lock isn't held while drawing.
struct Row {
    app: AppInfo,
    installed: Option<InstalledInfo>,
    latest: Option<LatestInfo>,
    update: bool,
    job: Option<Job>,
    error: Option<String>,
    muted: bool,
}

struct InstalledInfo {
    version: String,
    can_launch: bool,
}

struct LatestInfo {
    version: String,
    date: String,
    notes: String,
    url: String,
}

struct Snapshot {
    rows: Vec<Row>,
    jobs: Vec<Job>,
    checking: bool,
    last_check: Option<u64>,
    check_error: Option<String>,
    updates: usize,
    theme: ThemeChoice,
}

/// Clicks are collected while drawing and carried out afterwards.
enum Command {
    Enqueue(String, Action),
    Launch(String),
    Cancel(u64),
    ShowNotes(String),
    AskUninstall(String),
    Open(String),
    SetMuted(String, bool),
    SetTheme(ThemeChoice),
}

pub struct ManagerApp {
    mgr: Arc<Manager>,
    tab: Tab,
    notes: Option<String>,
    confirm_uninstall: Option<String>,
    /// Text, whether it's an error, and when it was shown.
    message: Option<(String, bool, Instant)>,
    logo: egui::TextureHandle,
    icons: IconCache,
    data_dir: PathBuf,
    applied_theme: ThemeChoice,
    hide_on_first_frame: bool,
    hidden: bool,
}

impl ManagerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mgr: Arc<Manager>, start_hidden: bool) -> Self {
        let ctx = &cc.egui_ctx;
        let theme = mgr.lock().settings.theme;
        theme::install(ctx, theme);
        let logo = ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([128, 128], &logo::rgba(128, false)),
            egui::TextureOptions::LINEAR,
        );
        ManagerApp {
            data_dir: mgr.paths.data.clone(),
            mgr,
            tab: Tab::Apps,
            notes: None,
            confirm_uninstall: None,
            message: None,
            logo,
            icons: IconCache::default(),
            applied_theme: theme,
            hide_on_first_frame: start_hidden,
            hidden: start_hidden,
        }
    }

    fn snapshot(&self) -> Snapshot {
        let st = self.mgr.lock();
        let rows = st
            .catalog
            .iter()
            .map(|app| Row {
                installed: st.installed.get(&app.id).map(|i| InstalledInfo {
                    version: i.version.clone(),
                    can_launch: i.exe.is_some(),
                }),
                latest: st.latest(&app.id).map(|r| LatestInfo {
                    version: r.version(),
                    date: r.date().to_owned(),
                    notes: r.body.clone().unwrap_or_default(),
                    url: r.html_url.clone(),
                }),
                update: st.has_update(&app.id),
                job: st.active_job(&app.id).cloned(),
                // Transient errors (offline, rate limit) are shown once in the status bar;
                // a row only shows its error when there's no cached release to show instead.
                error: st.check_errors.get(&app.id).filter(|_| st.latest(&app.id).is_none()).cloned(),
                muted: st.settings.muted.contains(&app.id),
                app: app.clone(),
            })
            .collect();
        Snapshot {
            rows,
            jobs: st.jobs.clone(),
            checking: st.checking,
            last_check: st.store.last_check,
            check_error: st.check_error.clone(),
            updates: st.updates().len(),
            theme: st.settings.theme,
        }
    }

    fn on_close(&mut self, ctx: &egui::Context) {
        // Never let eframe close the window itself: quitting goes through the
        // tray thread so the tray icon is removed.
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if !self.mgr.lock().settings.minimize_to_tray {
            self.mgr.quit();
            return;
        }
        if !tray::is_running() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.hidden = true;
        let first_time = !std::mem::replace(&mut self.mgr.lock().store.tray_hint_shown, true);
        if first_time {
            self.mgr.save_store();
            notify::show(
                format!("{APP_NAME} is still running"),
                "It keeps checking for updates in the background. Use the tray icon to open it again or to quit.",
            );
        }
    }

    fn run(&mut self, command: Command) {
        let result = match command {
            Command::Enqueue(id, action) => self.mgr.enqueue(&id, action),
            Command::Launch(id) => self.mgr.launch(&id),
            Command::Cancel(job) => Ok(self.mgr.cancel(job)),
            Command::ShowNotes(id) => Ok(self.notes = Some(id)),
            Command::AskUninstall(id) => Ok(self.confirm_uninstall = Some(id)),
            Command::Open(url) => Ok(platform::open(url)),
            Command::SetMuted(id, muted) => Ok(self.mgr.set_muted(&id, muted)),
            Command::SetTheme(choice) => {
                let mut settings = self.mgr.lock().settings.clone();
                settings.theme = choice;
                self.mgr.update_settings(settings)
            }
        };
        if let Err(e) = result {
            self.message = Some((e, true, Instant::now()));
        }
    }

    /// The brand bar and the tab strip, like the site's top navigation.
    fn nav(&mut self, ctx: &egui::Context, snap: &Snapshot, p: &Palette, commands: &mut Vec<Command>) {
        egui::TopBottomPanel::top("nav").frame(egui::Frame::new().fill(p.bg)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = ui.available_width();
            let painter = ui.painter().clone();

            let (bar, _) = ui.allocate_exact_size(Vec2::new(width, 46.0), Sense::hover());
            let logo = Rect::from_center_size(Pos2::new(bar.left() + PAD + 11.0, bar.center().y), Vec2::splat(22.0));
            egui::Image::new((self.logo.id(), logo.size())).paint_at(ui, logo);
            let title = galley(ui, "ARTCRAFT APPS", FontId::new(15.0, theme::display()), p.strong, 0.2);
            let title_pos = Pos2::new(logo.right() + 10.0, bar.center().y - title.size().y / 2.0);
            let sub = galley(ui, "MANAGER", hud_font(), p.muted, 1.5);
            painter.galley(
                Pos2::new(title_pos.x + title.size().x + 8.0, bar.center().y - sub.size().y / 2.0 + 0.5),
                sub,
                p.muted,
            );
            painter.galley(title_pos, title, p.strong);

            let controls = bar.shrink2(Vec2::new(PAD, 0.0));
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(controls).layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if theme_toggle(ui, p).clicked() {
                        let next = if p.dark { ThemeChoice::Light } else { ThemeChoice::Dark };
                        commands.push(Command::SetTheme(next));
                    }
                    let check = ui
                        .add_enabled(!snap.checking, egui::Button::new(spaced("Check", 1.2)))
                        .on_hover_text("Check GitHub for new versions now");
                    if check.clicked() {
                        self.mgr.check_now();
                    }
                },
            );
            painter.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0_f32, p.line));

            let (strip, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
            let active = snap.jobs.iter().filter(|j| j.status.is_active()).count();
            let tabs = [
                (Tab::Apps, if snap.updates > 0 { format!("Apps · {} new", snap.updates) } else { "Apps".to_owned() }),
                (Tab::Queue, if active > 0 { format!("Queue · {active}") } else { "Queue".to_owned() }),
                (Tab::Settings, "Settings".to_owned()),
            ];
            let cell = strip.width() / tabs.len() as f32;
            for (i, (tab, label)) in tabs.into_iter().enumerate() {
                let r = Rect::from_min_size(strip.min + Vec2::new(cell * i as f32, 0.0), Vec2::new(cell, strip.height()));
                let response = ui.interact(r, ui.id().with(("tab", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                let selected = self.tab == tab;
                response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
                if selected {
                    painter.rect_filled(r, 0.0, p.invert_bg);
                } else if response.hovered() {
                    painter.rect_filled(r, 0.0, p.hover);
                }
                let color = if selected { p.invert_fg } else { p.muted };
                let text = galley(ui, &label.to_uppercase(), FontId::new(10.5, theme::mono_bold()), color, 1.4);
                painter.galley(r.center() - text.size() / 2.0, text, color);
                if i > 0 {
                    painter.vline(r.left(), r.y_range(), Stroke::new(1.0_f32, p.line));
                }
                if response.clicked() {
                    self.tab = tab;
                }
            }
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context, snap: &Snapshot, p: &Palette) {
        if let Some((_, false, at)) = &self.message {
            if at.elapsed() > Duration::from_secs(6) {
                self.message = None;
            }
        }
        let frame = egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(PAD as i8, 7));
        egui::TopBottomPanel::bottom("status").frame(frame).show(ctx, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if self.message.is_some() {
                    if ui.small_button("✖").on_hover_text("Dismiss").clicked() {
                        self.message = None;
                    }
                } else if snap.updates > 0 {
                    let n = snap.updates;
                    ui.label(hud(&format!("{n} update{}", if n == 1 { "" } else { "s" }), p.update));
                } else if snap.last_check.is_some() && snap.check_error.is_none() {
                    ui.label(hud("Up to date", p.success));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if let Some((text, is_error, _)) = &self.message {
                        let color = if *is_error { p.danger } else { p.ink };
                        ui.add(egui::Label::new(RichText::new(text).small().color(color)).truncate()).on_hover_text(text);
                    } else if snap.checking {
                        ui.add(egui::Spinner::new().size(10.0).color(p.muted));
                        ui.label(hud("Checking GitHub…", p.muted));
                    } else if let Some(error) = &snap.check_error {
                        ui.add(egui::Label::new(RichText::new(error).small().color(p.danger)).truncate())
                            .on_hover_text(error);
                    } else if let Some(t) = snap.last_check {
                        ui.label(hud(&format!("Checked {}", util::ago(t)), p.muted));
                    }
                });
            });
        });
    }

    fn apps_tab(&mut self, ui: &mut egui::Ui, snap: &Snapshot, p: &Palette, commands: &mut Vec<Command>) {
        let updates: Vec<&Row> = snap.rows.iter().filter(|r| r.installed.is_some() && r.update).collect();
        let installed: Vec<&Row> = snap.rows.iter().filter(|r| r.installed.is_some() && !r.update).collect();
        let available: Vec<&Row> = snap.rows.iter().filter(|r| r.installed.is_none()).collect();

        let mut index = 0;
        for (title, rows) in [("Updates", &updates), ("Installed", &installed), ("Available", &available)] {
            if rows.is_empty() {
                continue;
            }
            index += 1;
            let action = (title == "Updates").then_some("Update all");
            if section_bar(ui, p, index, &format!("{title} ({})", rows.len()), action) {
                self.mgr.update_all();
            }
            for row in rows {
                app_row(ui, p, row, &snap.jobs, &mut self.icons, &self.data_dir, commands);
            }
        }
        if snap.rows.iter().all(|r| r.latest.is_none()) && snap.checking {
            note(ui, p, "Looking up the apps on GitHub…");
        }
    }

    fn queue_tab(&mut self, ui: &mut egui::Ui, snap: &Snapshot, p: &Palette, commands: &mut Vec<Command>) {
        let active: Vec<&Job> = snap.jobs.iter().filter(|j| j.status.is_active()).collect();
        let finished: Vec<&Job> = snap.jobs.iter().rev().filter(|j| j.status.is_finished()).collect();

        section_bar(ui, p, 1, &format!("In progress ({})", active.len()), None);
        if active.is_empty() {
            note(ui, p, "Nothing queued. Installs and updates download one at a time.");
        }
        for job in active {
            row_frame(ui, p, |ui| {
                ui.horizontal(|ui| {
                    app_icon(ui, &job.app, 30.0, &mut self.icons, &self.data_dir);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if cancellable(job) && ui.button(spaced("Cancel", 1.2)).clicked() {
                            commands.push(Command::Cancel(job.id));
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.vertical(|ui| {
                                ui.add(egui::Label::new(app_title(&job.app.name, &job.version, p)).truncate());
                                ui.add(egui::Label::new(hud(&package_text(job), p.faint)).truncate());
                            });
                        });
                    });
                });
                ui.add_space(5.0);
                job_progress(ui, p, job, &snap.jobs, color(job.app.colors.tile));
            });
        }

        let clear = (!finished.is_empty()).then_some("Clear");
        if section_bar(ui, p, 2, "History", clear) {
            self.mgr.clear_finished();
        }
        if finished.is_empty() {
            note(ui, p, "Finished installs are listed here until you quit.");
        }
        for job in finished {
            let (mark, mark_color, text) = match &job.status {
                Status::Done(m) => ("✔", p.success, m.clone()),
                Status::Failed(e) => ("✖", p.danger, e.clone()),
                _ => ("–", p.faint, "Cancelled".to_owned()),
            };
            row_frame(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(mark).color(mark_color));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if let Some(t) = job.finished_at {
                            ui.label(hud(&util::ago(t), p.faint));
                        }
                        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                            ui.vertical(|ui| {
                                let title = format!("{} · {}", job.action.label(), job.app.name);
                                ui.add(egui::Label::new(app_title(&title, &job.version, p)).truncate());
                                ui.add(egui::Label::new(RichText::new(text).small().color(p.muted)).wrap());
                            });
                        });
                    });
                });
            });
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let mut s = self.mgr.lock().settings.clone();
        let before = s.clone();
        let mgr = self.mgr.clone();

        section_bar(ui, p, 1, "Appearance", None);
        padded(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Theme").color(p.ink));
                ui.add_space(8.0);
                segmented(
                    ui,
                    p,
                    &mut s.theme,
                    &[(ThemeChoice::System, "System"), (ThemeChoice::Light, "Light"), (ThemeChoice::Dark, "Dark")],
                );
            });
        });

        section_bar(ui, p, 2, "General", None);
        padded(ui, |ui| {
            setting(ui, p, &mut s.start_at_login, "Start when I sign in to Windows",
                "Opens minimized in the system tray and checks for updates in the background.");
            setting(ui, p, &mut s.minimize_to_tray, "Keep running in the tray when closed",
                "When off, closing the window quits the app.");
        });

        section_bar(ui, p, 3, "Updates", None);
        padded(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Check every").color(p.ink));
                egui::ComboBox::from_id_salt("interval")
                    .selected_text(spaced(&interval_label(s.check_interval_hours), 1.2))
                    .show_ui(ui, |ui| {
                        for hours in [1, 2, 4, 8, 12, 24] {
                            ui.selectable_value(&mut s.check_interval_hours, hours, spaced(&interval_label(hours), 1.2));
                        }
                    });
            });
            ui.add_space(4.0);
            setting(ui, p, &mut s.notify_updates, "Notify me when updates are available",
                "Once per new version. Turn it off for one app from its ··· menu.");
            setting(ui, p, &mut s.notify_installs, "Notify me when an install finishes or fails", "");
            setting(ui, p, &mut s.auto_install_updates, "Install updates automatically",
                "Updates are queued as soon as they're found. Windows asks for admin permission before each one installs.");
        });

        section_bar(ui, p, 4, "Installing", None);
        padded(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(r"Apps install for all users in C:\Program Files from their official installers, with a Start menu entry. Windows asks for admin permission each time.")
                        .small()
                        .color(p.muted),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            setting(ui, p, &mut s.keep_installers, "Keep downloaded installers", "Otherwise they're deleted once installed.");
            if ui.button(spaced("Open downloads folder", 1.2)).clicked() {
                platform::open(&mgr.paths.downloads);
            }
        });

        section_bar(ui, p, 5, "About", None);
        padded(ui, |ui| {
            ui.label(app_title(APP_NAME, env!("CARGO_PKG_VERSION"), p));
            ui.label(hud(&format!("Windows {} · Release data from github.com/storytold", mgr.arch.label()), p.faint));
            ui.add(
                egui::Label::new(
                    RichText::new("App icons and colours are the Craft apps' own. Fonts: Inter, Archivo and Geist Mono (SIL Open Font License).")
                        .small()
                        .color(p.muted),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(spaced("Open data folder", 1.2)).on_hover_text(mgr.paths.data.display().to_string()).clicked() {
                    platform::open(&mgr.paths.data);
                }
                if ui.button(spaced("Quit", 1.2)).clicked() {
                    mgr.quit();
                }
            });
        });

        if s != before {
            if let Err(e) = self.mgr.update_settings(s) {
                self.message = Some((e, true, Instant::now()));
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, snap: &Snapshot, p: &Palette, commands: &mut Vec<Command>) {
        if let Some(id) = self.notes.clone() {
            let found = snap.rows.iter().find(|r| r.app.id == id).and_then(|r| Some((&r.app, r.latest.as_ref()?)));
            let mut open = found.is_some();
            if let Some((app, latest)) = found {
                egui::Window::new(RichText::new(format!("{} {}", app.name, latest.version)).color(p.strong))
                    .id(egui::Id::new("release-notes"))
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .default_width(360.0)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(hud(&format!("Released {}", latest.date), p.muted));
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| release_notes(ui, p, &latest.notes));
                        ui.add_space(10.0);
                        if ui.button(spaced("View on GitHub", 1.2)).clicked() {
                            commands.push(Command::Open(latest.url.clone()));
                        }
                    });
            }
            if !open {
                self.notes = None;
            }
        }

        if let Some(id) = self.confirm_uninstall.clone() {
            let Some(row) = snap.rows.iter().find(|r| r.app.id == id) else {
                self.confirm_uninstall = None;
                return;
            };
            let modal = egui::Modal::new(egui::Id::new("confirm-uninstall")).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(RichText::new(format!("Uninstall {}?", row.app.name)).font(FontId::new(17.0, theme::display())).color(p.strong));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new("Windows Installer removes it and may ask for admin permission. Files you saved aren't affected.")
                            .color(p.muted),
                    )
                    .wrap(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let uninstall = egui::Button::new(spaced("Uninstall", 1.2).color(Color32::WHITE)).fill(p.danger);
                    if ui.add(uninstall).clicked() {
                        commands.push(Command::Enqueue(id.clone(), Action::Uninstall));
                        self.confirm_uninstall = None;
                    }
                    if ui.button(spaced("Cancel", 1.2)).clicked() {
                        self.confirm_uninstall = None;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_uninstall = None;
            }
        }
    }
}

impl eframe::App for ManagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.hide_on_first_frame) {
            // eframe always shows the window after its first frame; this hides it again straight after.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else if self.hidden && platform::is_main_window_visible() {
            // Shown from the tray or by a second launch: bring winit's view of it back in sync.
            self.hidden = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.on_close(ctx);
        }

        let snap = self.snapshot();
        if snap.theme != self.applied_theme {
            ctx.set_theme(theme::preference(snap.theme));
            self.applied_theme = snap.theme;
        }
        let p = Palette::of(ctx);
        let mut commands = Vec::new();
        self.nav(ctx, &snap, &p, &mut commands);
        self.status_bar(ctx, &snap, &p);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match self.tab {
                    Tab::Apps => self.apps_tab(ui, &snap, &p, &mut commands),
                    Tab::Queue => self.queue_tab(ui, &snap, &p, &mut commands),
                    Tab::Settings => self.settings_tab(ui, &p),
                }
            });
        });
        self.dialogs(ctx, &snap, &p, &mut commands);
        for command in commands {
            self.run(command);
        }

        let busy = snap.checking || snap.jobs.iter().any(|j| j.status.is_active());
        ctx.request_repaint_after(Duration::from_secs(if busy { 1 } else { 30 }));
    }
}

fn app_row(
    ui: &mut egui::Ui,
    p: &Palette,
    row: &Row,
    jobs: &[Job],
    icons: &mut IconCache,
    data_dir: &Path,
    commands: &mut Vec<Command>,
) {
    let ink = color(row.app.colors.ink(p.dark));
    row_frame(ui, p, |ui| {
        ui.horizontal(|ui| {
            app_icon(ui, &row.app, 38.0, icons, data_dir);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                more_menu(ui, row, commands);
                match &row.job {
                    Some(job) => {
                        if cancellable(job) && ui.button(spaced("Cancel", 1.2)).clicked() {
                            commands.push(Command::Cancel(job.id));
                        }
                    }
                    None => {
                        let can_launch = row.installed.as_ref().is_some_and(|i| i.can_launch);
                        if can_launch && !row.update && ui.button(spaced("Open", 1.2)).clicked() {
                            commands.push(Command::Launch(row.app.id.clone()));
                        }
                        if row.update {
                            if primary(ui, p, "Update").clicked() {
                                commands.push(Command::Enqueue(row.app.id.clone(), Action::Update));
                            }
                        } else if row.installed.is_none() && row.latest.is_some() && ui.button(spaced("Install", 1.2)).clicked() {
                            commands.push(Command::Enqueue(row.app.id.clone(), Action::Install));
                        }
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.vertical(|ui| {
                        let name = RichText::new(&row.app.name).font(FontId::new(15.0, theme::display())).color(p.strong);
                        ui.add(egui::Label::new(name).truncate()).on_hover_text(&row.app.description);
                        ui.add(egui::Label::new(details(row, p, ink)).truncate());
                    });
                });
            });
        });
        if let Some(job) = &row.job {
            ui.add_space(5.0);
            job_progress(ui, p, job, jobs, color(row.app.colors.tile));
        } else if let Some(error) = &row.error {
            ui.add_space(3.0);
            ui.add(egui::Label::new(RichText::new(error).small().color(p.danger)).truncate()).on_hover_text(error);
        }
    });
}

/// "V0.3.0 › V0.5.0 · IMAGE EDITOR" under an app's name. Versions come first
/// so that a long category is what gets cut off in a narrow window.
fn details(row: &Row, p: &Palette, ink: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut add = |text: &str, color: Color32| job.append(&text.to_uppercase(), 0.0, hud_format(color));
    match (&row.installed, &row.latest) {
        (Some(i), Some(l)) if row.update => {
            add(&format!("v{} › ", i.version), p.muted);
            add(&format!("v{}", l.version), p.update);
        }
        (Some(i), _) => add(&format!("v{}", i.version), p.muted),
        (None, Some(l)) => add(&format!("v{}", l.version), p.muted),
        (None, None) => add("No release yet", p.faint),
    }
    if row.installed.is_some() && row.muted {
        add(" · Muted", p.faint);
    }
    if !row.app.category.is_empty() {
        add(" · ", p.faint);
        add(&row.app.category, ink);
    }
    job
}

fn more_menu(ui: &mut egui::Ui, row: &Row, commands: &mut Vec<Command>) {
    let id = &row.app.id;
    ui.menu_button(RichText::new("···"), |ui| {
        ui.set_min_width(190.0);
        let item = |ui: &mut egui::Ui, label: &str| {
            let clicked = ui.button(spaced(label, 1.2)).clicked();
            if clicked {
                ui.close_menu();
            }
            clicked
        };
        // Update rows have no room for an Open button, so it lives here.
        if row.update && row.installed.as_ref().is_some_and(|i| i.can_launch) && item(ui, "Open") {
            commands.push(Command::Launch(id.clone()));
        }
        if row.latest.is_some() && item(ui, "Release notes") {
            commands.push(Command::ShowNotes(id.clone()));
        }
        if item(ui, "GitHub") {
            commands.push(Command::Open(row.app.repo_url()));
        }
        if let Some(home) = &row.app.homepage {
            if item(ui, "Website") {
                commands.push(Command::Open(home.clone()));
            }
        }
        if row.installed.is_some() {
            ui.separator();
            let mut notify = !row.muted;
            if ui.checkbox(&mut notify, RichText::new("Notify me about updates").text_style(TextStyle::Body)).changed() {
                commands.push(Command::SetMuted(id.clone(), !notify));
            }
            if row.job.is_none() {
                if !row.update && row.latest.is_some() && item(ui, "Reinstall") {
                    commands.push(Command::Enqueue(id.clone(), Action::Reinstall));
                }
                if item(ui, "Uninstall…") {
                    commands.push(Command::AskUninstall(id.clone()));
                }
            }
        }
    });
}

fn cancellable(job: &Job) -> bool {
    matches!(job.status, Status::Queued | Status::Downloading | Status::ReadyToInstall)
}

/// How many jobs will download before `job` does.
fn jobs_ahead(jobs: &[Job], job: &Job) -> usize {
    jobs.iter()
        .filter(|j| j.id < job.id && matches!(j.status, Status::Queued | Status::Downloading))
        .count()
}

fn job_progress(ui: &mut egui::Ui, p: &Palette, job: &Job, jobs: &[Job], bar_color: Color32) {
    let (text, fraction) = match &job.status {
        Status::Queued => match jobs_ahead(jobs, job) {
            0 => ("Starting download…".to_owned(), Some(0.0)),
            n => (format!("Queued · {n} ahead"), Some(0.0)),
        },
        Status::Downloading => (
            format!(
                "{} of {} · {}/s",
                human_bytes(job.downloaded),
                human_bytes(job.total),
                human_bytes(job.speed as u64)
            ),
            Some(job.downloaded as f32 / job.total.max(1) as f32),
        ),
        Status::ReadyToInstall if job.action == Action::Uninstall => ("Waiting to uninstall".to_owned(), None),
        Status::ReadyToInstall => ("Downloaded · waiting to install".to_owned(), Some(1.0)),
        Status::Installing if job.action == Action::Uninstall => ("Uninstalling…".to_owned(), None),
        Status::Installing => ("Installing… approve the Windows prompt".to_owned(), None),
        Status::Done(m) | Status::Failed(m) => (m.clone(), None),
        Status::Cancelled => ("Cancelled".to_owned(), None),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if job.status == Status::Installing {
            ui.add(egui::Spinner::new().size(10.0).color(p.muted));
        }
        ui.add(egui::Label::new(hud(&text, p.muted)).truncate());
    });
    if let Some(fraction) = fraction {
        ui.add_space(4.0);
        let (track, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
        ui.painter().rect_filled(track, 0.0, p.line);
        let done = Rect::from_min_size(track.min, Vec2::new(track.width() * fraction.clamp(0.0, 1.0), track.height()));
        ui.painter().rect_filled(done, 0.0, bar_color);
    }
}

fn package_text(job: &Job) -> String {
    let Some(asset) = &job.asset else { return job.action.label().to_owned() };
    let mut text = format!("{} · {}", asset.name, human_bytes(asset.size));
    if job.verified {
        text.push_str(" · SHA-256 verified");
    }
    text
}

/// Release notes are Markdown; this shows them as tidy plain text.
fn release_notes(ui: &mut egui::Ui, p: &Palette, notes: &str) {
    if notes.trim().is_empty() {
        ui.label(RichText::new("No release notes.").color(p.muted));
        return;
    }
    for line in notes.lines() {
        let line = line.trim_end().replace("**", "");
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            ui.add_space(4.0);
        } else if let Some(heading) = trimmed.strip_prefix('#') {
            ui.label(RichText::new(heading.trim_start_matches('#').trim()).font(FontId::new(14.0, theme::display())).color(p.strong));
        } else if let Some(item) = trimmed.strip_prefix("* ").or_else(|| trimmed.strip_prefix("- ")) {
            ui.add(egui::Label::new(RichText::new(format!("•  {item}")).color(p.ink)).wrap());
        } else {
            ui.add(egui::Label::new(RichText::new(trimmed).color(p.ink)).wrap());
        }
    }
}

/// A full-width row with padding, a hover tint and a hairline underneath.
fn row_frame(ui: &mut egui::Ui, p: &Palette, add: impl FnOnce(&mut egui::Ui)) {
    let background = ui.painter().add(egui::Shape::Noop);
    let rect = egui::Frame::new()
        .inner_margin(Margin::symmetric(PAD as i8, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(10.0, 2.0);
            add(ui);
        })
        .response
        .rect;
    if ui.rect_contains_pointer(rect) {
        ui.painter().set(background, egui::Shape::rect_filled(rect, 0.0, p.hover));
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, p.line));
}

/// A bar like the site's "01 / CRAFTING APPS". Returns true when its action
/// (shown on the right) is clicked.
fn section_bar(ui: &mut egui::Ui, p: &Palette, index: usize, title: &str, action: Option<&str>) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, p.hover);
    if index > 1 {
        painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0_f32, p.line));
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, p.line));
    let label = galley(ui, &format!("{index:02} / {}", title.to_uppercase()), hud_font(), p.muted, 1.5);
    painter.galley(Pos2::new(rect.left() + PAD, rect.center().y - label.size().y / 2.0), label, p.muted);

    let Some(action) = action else { return false };
    let text = galley(ui, &action.to_uppercase(), FontId::new(10.0, theme::mono_bold()), p.accent_ink, 1.5);
    let text_rect = Rect::from_min_size(
        Pos2::new(rect.right() - PAD - text.size().x, rect.center().y - text.size().y / 2.0),
        text.size(),
    );
    let response = ui
        .interact(text_rect.expand(6.0), ui.id().with(("section-action", index)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, action));
    if response.hovered() {
        painter.hline(text_rect.x_range(), text_rect.bottom() + 1.0, Stroke::new(1.0_f32, p.accent_ink));
    }
    painter.galley(text_rect.min, text, p.accent_ink);
    response.clicked()
}

fn note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    egui::Frame::new().inner_margin(Margin::symmetric(PAD as i8, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).small().color(p.faint)).wrap());
    });
}

fn padded(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .inner_margin(Margin { left: PAD as i8, right: PAD as i8, top: 12, bottom: 14 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui);
        });
}

fn setting(ui: &mut egui::Ui, p: &Palette, value: &mut bool, label: &str, help: &str) {
    ui.checkbox(value, RichText::new(label).text_style(TextStyle::Body).color(p.ink));
    if !help.is_empty() {
        ui.indent(label, |ui| {
            ui.add(egui::Label::new(RichText::new(help).small().color(p.muted)).wrap());
        });
    }
    ui.add_space(2.0);
}

/// Equal-width options in a bordered strip; the chosen one is inverted, like
/// the site's active navigation item.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, p: &Palette, value: &mut T, options: &[(T, &str)]) {
    let cell = Vec2::new(72.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(cell.x * options.len() as f32, cell.y), Sense::hover());
    let painter = ui.painter().clone();
    for (i, (option, label)) in options.iter().enumerate() {
        let r = Rect::from_min_size(rect.min + Vec2::new(cell.x * i as f32, 0.0), cell);
        let response = ui.interact(r, ui.id().with(("segment", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let selected = *value == *option;
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, *label));
        if selected {
            painter.rect_filled(r, 0.0, p.invert_bg);
        } else if response.hovered() {
            painter.rect_filled(r, 0.0, p.hover);
        }
        let color = if selected { p.invert_fg } else { p.ink };
        let text = galley(ui, &label.to_uppercase(), FontId::new(10.5, theme::mono_bold()), color, 1.2);
        painter.galley(r.center() - text.size() / 2.0, text, color);
        if i > 0 {
            painter.vline(r.left(), r.y_range(), Stroke::new(1.0_f32, p.line_strong));
        }
        if response.clicked() {
            *value = *option;
        }
    }
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, p.line_strong), StrokeKind::Inside);
}

/// A half-filled circle, the site's light/dark switch.
fn theme_toggle(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Toggle light or dark theme"));
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 0.0, p.hover);
    }
    painter.rect_stroke(rect, 0.0, visuals.bg_stroke, StrokeKind::Inside);
    let (c, r, color) = (rect.center(), 6.0, visuals.fg_stroke.color);
    painter.circle_stroke(c, r, Stroke::new(1.5_f32, color));
    let half: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 16.0;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(half, color, Stroke::NONE));
    response
        .on_hover_text(if p.dark { "Switch to the light theme" } else { "Switch to the dark theme" })
        .on_hover_cursor(CursorIcon::PointingHand)
}

fn app_icon(ui: &mut egui::Ui, app: &AppInfo, size: f32, icons: &mut IconCache, data_dir: &Path) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if let Some(texture) = icons.get(ui.ctx(), &app.id, data_dir) {
        egui::Image::new((texture.id(), rect.size())).paint_at(ui, rect);
    } else {
        // No icon published: a tile in the app's colour with its initial.
        ui.painter().rect_filled(rect, size * 0.22, color(app.colors.tile));
        let initial: String = app.name.chars().take(1).collect();
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, initial, FontId::new(size * 0.5, theme::display()), Color32::WHITE);
    }
}

/// "PhotoCraft 0.5.0" with the version de-emphasised.
fn app_title(name: &str, version: &str, p: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(name, 0.0, TextFormat { font_id: FontId::new(13.5, theme::display()), color: p.strong, ..Default::default() });
    job.append(&format!("v{version}").to_uppercase(), 8.0, hud_format(p.muted));
    job
}

fn primary(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(egui::Button::new(spaced(text, 1.2).color(p.invert_fg)).fill(p.invert_bg))
}

/// Uppercase text with the site's wide letter spacing, for buttons.
fn spaced(text: &str, spacing: f32) -> RichText {
    RichText::new(text.to_uppercase()).extra_letter_spacing(spacing)
}

/// Uppercase mono label, the site's "HUD label".
fn hud(text: &str, color: Color32) -> RichText {
    RichText::new(text.to_uppercase()).text_style(theme::hud()).color(color).extra_letter_spacing(1.3)
}

fn hud_font() -> FontId {
    FontId::new(10.0, FontFamily::Monospace)
}

fn hud_format(color: Color32) -> TextFormat {
    TextFormat { font_id: hud_font(), color, extra_letter_spacing: 1.3, ..Default::default() }
}

fn galley(ui: &egui::Ui, text: &str, font: FontId, color: Color32, spacing: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.append(text, 0.0, TextFormat { font_id: font, color, extra_letter_spacing: spacing, ..Default::default() });
    ui.fonts(|f| f.layout_job(job))
}

fn color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn interval_label(hours: u32) -> String {
    match hours {
        1 => "hour".to_owned(),
        24 => "day".to_owned(),
        h => format!("{h} hours"),
    }
}
