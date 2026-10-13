//! A file dialog drawn in the window, sized for fingers: for hosts whose system has no file dialog
//! the app can wait on (Android). The host hands [`FileBrowser::starter`] to
//! [`crate::Services::start_pick`] and draws [`FileBrowser::show`] after the app each frame; the
//! answer goes back like a system dialog's shown off the UI thread ([`crate::picks`]).
//!
//! It lists one folder at a time: folders first, then the files the chosen type shows. Places
//! (the host's: Documents, Download…) are a row of buttons on top. Open picks a file with one tap;
//! Place picks several; Save takes a name (a file tapped gives its name), and asks once more before
//! replacing a file; Folder picks the folder shown.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use egui::{Color32, CornerRadius, Sense, Ui, vec2};

use crate::picks::{PickAnswer, PickRequest, StartPick};
use crate::theme::Tokens;
use crate::{FilePick, icons, widgets};

/// Height of a row in the list, and of the buttons: a fingertip.
const ROW: f32 = 44.0;
/// Most entries listed from one folder (a folder with more lists the first ones).
const MAX_ENTRIES: usize = 5000;

/// A folder offered in the places row.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
}

/// What the host can do when a folder can't be read: ask the system for access to all files.
pub struct Access {
    /// Whether the app has it now.
    pub granted: Box<dyn Fn() -> bool>,
    /// Ask for it (opens the system's settings page; the answer arrives when the user comes back).
    pub request: Box<dyn FnMut()>,
}

/// One entry of the folder shown.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    name: String,
    dir: bool,
    size: u64,
}

/// The dialog open now.
struct Browsing {
    request: PickRequest,
    reply: Sender<PickAnswer>,
    dir: PathBuf,
    entries: Vec<Entry>,
    /// Why the folder couldn't be listed.
    error: Option<String>,
    /// The file type shown (index into the pick's filters; past the end: every file).
    filter: usize,
    /// Save: the name typed.
    name: String,
    /// Save: the named file exists and the next tap on Save replaces it.
    confirm_replace: bool,
    /// Place: the files ticked.
    ticked: Vec<PathBuf>,
    /// The New Folder field while it is open.
    new_folder: Option<String>,
}

/// The in-window file dialog and where it starts.
#[derive(Default)]
pub struct FileBrowser {
    places: Vec<Place>,
    /// Where a dialog starts that names no folder: the last folder a dialog picked from, else the
    /// first place.
    last_dir: Option<PathBuf>,
    open: Option<Browsing>,
    pub access: Option<Access>,
    /// Whether [`Access::granted`] said yes last frame: the folder is listed again when it turns
    /// to yes (the user comes back from the system's settings).
    granted_before: bool,
}

impl FileBrowser {
    pub fn new(places: Vec<Place>) -> Self {
        Self { places, ..Self::default() }
    }

    /// The [`crate::Services::start_pick`] that opens `browser`, waking `ctx` to draw it.
    pub fn starter(browser: &Rc<RefCell<FileBrowser>>, ctx: egui::Context) -> StartPick {
        let browser = browser.clone();
        Box::new(move |request: PickRequest| {
            let (tx, rx) = channel();
            browser.borrow_mut().start(request, tx);
            ctx.request_repaint();
            Some(rx)
        })
    }

    /// A dialog is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open the dialog for `request`, answering on `reply`. One already open is cancelled.
    pub fn start(&mut self, request: PickRequest, reply: Sender<PickAnswer>) {
        self.cancel();
        let pick = pick_of(&request);
        let dir = pick
            .and_then(|p| p.folder.as_deref())
            .map(PathBuf::from)
            .filter(|d| d.is_dir())
            .or_else(|| self.last_dir.clone().filter(|d| d.is_dir()))
            .or_else(|| self.places.iter().map(|p| p.path.clone()).find(|d| d.is_dir()))
            .unwrap_or_else(|| PathBuf::from("/"));
        let name = match &request {
            PickRequest::Save(p) => p.name.clone(),
            _ => String::new(),
        };
        let mut b = Browsing {
            request,
            reply,
            dir: dir.clone(),
            entries: vec![],
            error: None,
            filter: 0,
            name,
            confirm_replace: false,
            ticked: vec![],
            new_folder: None,
        };
        b.go(dir);
        self.open = Some(b);
    }

    /// Close the dialog open, answering nothing (cancelled).
    pub fn cancel(&mut self) {
        if let Some(b) = self.open.take() {
            // The app may have stopped waiting (a panic it recovered from): nothing to tell then.
            let _ = b.reply.send(vec![]);
        }
    }

    /// Answer `paths` and close.
    fn answer(&mut self, paths: Vec<PathBuf>) {
        let Some(b) = self.open.take() else { return };
        self.last_dir = Some(b.dir.clone());
        let _ = b.reply.send(paths.iter().map(|p| p.to_string_lossy().to_string()).collect());
    }

    /// Draw the open dialog, if any.
    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(b) = &mut self.open else { return };
        let t = Tokens::get(ctx);
        let screen = ctx.content_rect();
        let size = vec2((screen.width() - 16.0).max(200.0), (screen.height() - 24.0).max(240.0));
        let frame = egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(8)).inner_margin(12);
        let mut outcome = Outcome::None;
        let granted = self.access.as_ref().is_none_or(|a| (a.granted)());
        if granted && !self.granted_before {
            let dir = b.dir.clone();
            b.go(dir);
        }
        self.granted_before = granted;
        let modal = egui::Modal::new(egui::Id::new("file-browser")).frame(frame).show(ctx, |ui| {
            ui.set_width(size.x - 24.0);
            ui.set_height(size.y - 24.0);
            outcome = body(ui, b, &self.places, granted, &t);
        });
        if modal.should_close() && matches!(outcome, Outcome::None) {
            outcome = Outcome::Cancel;
        }
        if !matches!(outcome, Outcome::None) {
            // The app takes the answer next frame.
            ctx.request_repaint();
        }
        match outcome {
            Outcome::None => {}
            Outcome::Cancel => self.cancel(),
            Outcome::Answer(paths) => self.answer(paths),
            Outcome::Access => {
                if let Some(a) = &mut self.access {
                    (a.request)();
                }
            }
        }
    }
}

/// What a frame of the dialog did.
enum Outcome {
    None,
    Cancel,
    Answer(Vec<PathBuf>),
    /// The access button was tapped.
    Access,
}

fn pick_of(request: &PickRequest) -> Option<&FilePick> {
    match request {
        PickRequest::Open(p) | PickRequest::Save(p) => Some(p),
        PickRequest::OpenMany | PickRequest::Folder => None,
    }
}

impl Browsing {
    /// List `dir` and show it.
    fn go(&mut self, dir: PathBuf) {
        self.new_folder = None;
        self.confirm_replace = false;
        match list(&dir) {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e);
            }
        }
        self.dir = dir;
    }

    /// The pick's filters (none: every file).
    fn filters(&self) -> &[(&'static str, &'static [&'static str])] {
        pick_of(&self.request).map(|p| p.filters.as_slice()).unwrap_or(&[])
    }

    /// Whether the file `name` is listed under the type chosen.
    fn shows(&self, name: &str) -> bool {
        let Some((_, exts)) = self.filters().get(self.filter) else { return true };
        exts.is_empty() || ext_of(name).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(&e)))
    }

    /// Save: the path the name typed saves to, with the chosen type's extension when it has none.
    fn save_path(&self) -> Option<PathBuf> {
        let name = self.name.trim();
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return None;
        }
        let mut name = name.to_string();
        if ext_of(&name).is_none()
            && let Some(ext) = self.filters().get(self.filter).and_then(|(_, e)| e.first())
        {
            name = format!("{name}.{ext}");
        }
        Some(self.dir.join(name))
    }

    /// Save: choose filter `i`, giving the name that type's extension.
    fn choose_filter(&mut self, i: usize) {
        self.filter = i;
        if matches!(self.request, PickRequest::Save(_))
            && let Some(ext) = self.filters().get(i).and_then(|(_, e)| e.first())
        {
            let stem = match self.name.rsplit_once('.') {
                Some((stem, _)) if !stem.is_empty() => stem.to_string(),
                _ => self.name.clone(),
            };
            if !stem.is_empty() {
                self.name = format!("{stem}.{ext}");
            }
        }
        self.confirm_replace = false;
    }
}

/// The lower-case extension of `name`, if it has one.
fn ext_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// The entries of `dir`: folders first, then files, by name; hidden ones left out.
fn list(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().take(MAX_ENTRIES) {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // A link is followed: a link to a folder lists as one.
        let Ok(meta) = std::fs::metadata(e.path()) else { continue };
        out.push(Entry { name, dir: meta.is_dir(), size: meta.len() });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// A size in bytes for people.
fn size_text(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / K / K)
    } else {
        format!("{:.1} GB", f / K / K / K)
    }
}

/// A full-width button a finger can hit; `strong` for the dialog's main one.
fn big_button(ui: &mut Ui, text: &str, strong: bool, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(15.0), if strong { Color32::WHITE } else { t.text });
    let w = (galley.size().x + 36.0).max(96.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW), if enabled { Sense::click() } else { Sense::hover() });
    let bg = match (strong, enabled) {
        (true, true) => t.accent_strong,
        (true, false) => t.text_disabled,
        (false, _) if resp.hovered() => t.hover,
        _ => t.button,
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, if strong { Color32::WHITE } else { t.text });
    resp
}

/// The dialog's contents.
fn body(ui: &mut Ui, b: &mut Browsing, places: &[Place], granted: bool, t: &Tokens) -> Outcome {
    let mut out = Outcome::None;
    let title = match &b.request {
        PickRequest::Open(_) => tl!("Open"),
        PickRequest::OpenMany => tl!("Place"),
        PickRequest::Save(_) => tl!("Save"),
        PickRequest::Folder => tl!("Folder"),
    };
    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(18.0).strong().color(t.text_strong));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, "x", tl!("Cancel"), false, ROW).clicked() {
                out = Outcome::Cancel;
            }
        });
    });
    // Places.
    if !places.is_empty() {
        egui::ScrollArea::horizontal().id_salt("file-browser-places").show(ui, |ui| {
            ui.horizontal(|ui| {
                for p in places {
                    let here = b.dir.starts_with(&p.path);
                    let galley = ui.painter().layout_no_wrap(p.label.clone(), egui::FontId::proportional(14.0), t.text);
                    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 38.0), Sense::click());
                    ui.painter().rect_filled(rect, CornerRadius::same(19), if here { t.tool_active } else { t.button });
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, if here { t.text_strong } else { t.text });
                    if resp.clicked() {
                        b.go(p.path.clone());
                    }
                }
            });
        });
    }
    // The folder shown, Up and New Folder.
    ui.horizontal(|ui| {
        let parent = b.dir.parent().map(Path::to_path_buf);
        if widgets::icon_button_enabled(ui, "chevron-left", tl!("Up One Level"), false, parent.is_some(), ROW).clicked()
            && let Some(p) = parent
        {
            b.go(p);
        }
        let new_folder = !matches!(b.request, PickRequest::Open(_) | PickRequest::OpenMany);
        let room = ui.available_width() - if new_folder { ROW + 8.0 } else { 0.0 };
        let shown = b.dir.to_string_lossy().to_string();
        // The end of a long path matters most: cut from its start.
        let chars: Vec<char> = shown.chars().collect();
        let fit = ((room / 8.0) as usize).max(8);
        let text = if chars.len() > fit { format!("…{}", chars[chars.len() - fit + 1..].iter().collect::<String>()) } else { shown };
        ui.add_sized(vec2(room.max(40.0), ROW), egui::Label::new(egui::RichText::new(text).size(13.0).color(t.text_dim)).truncate());
        if new_folder && widgets::icon_button(ui, "dc-folder", tl!("New Folder"), b.new_folder.is_some(), ROW).clicked() {
            b.new_folder = if b.new_folder.is_some() { None } else { Some(String::new()) };
        }
    });
    if let Some(name) = &mut b.new_folder {
        let mut create = false;
        ui.horizontal(|ui| {
            let field =
                ui.add_sized(vec2((ui.available_width() - 120.0).max(80.0), 36.0), egui::TextEdit::singleline(name).hint_text(tl!("New Folder")));
            field.request_focus();
            create = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            create |= big_button(ui, tl!("OK"), true, !name.trim().is_empty()).clicked();
        });
        if create {
            let dir = b.dir.join(name.trim());
            match std::fs::create_dir(&dir) {
                Ok(()) => b.go(dir),
                Err(e) => b.error = Some(e.to_string()),
            }
        }
    }
    // The list takes the room the footer leaves.
    let list_height = (ui.available_height() - footer_height(b)).max(ROW * 2.0);
    if !granted && (b.error.is_some() || b.entries.is_empty()) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tl!("This folder can't be read without access to all files.")).color(t.warning));
            if big_button(ui, tl!("Allow Access"), true, true).clicked() {
                out = Outcome::Access;
            }
        });
    }
    if let Some(e) = &b.error {
        ui.label(egui::RichText::new(e).color(t.error));
    }
    let shown: Vec<Entry> = b.entries.iter().filter(|e| e.dir || b.shows(&e.name)).cloned().collect();
    egui::ScrollArea::vertical().id_salt(("file-browser-list", b.dir.clone())).max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        for e in &shown {
            let path = b.dir.join(&e.name);
            let ticked = b.ticked.contains(&path);
            let named = matches!(b.request, PickRequest::Save(_)) && !e.dir && b.name == e.name;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            let bg = if ticked || named {
                t.row_selected
            } else if resp.hovered() {
                t.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
            let icon = if e.dir { "dc-folder" } else { "file-plus" };
            icons::paint(
                ui,
                icon,
                egui::Rect::from_center_size(rect.left_center() + vec2(22.0, 0.0), vec2(22.0, 22.0)),
                if e.dir { t.accent } else { t.icon },
            );
            let right = if e.dir { String::new() } else { size_text(e.size) };
            let rg = ui.painter().layout_no_wrap(right, egui::FontId::proportional(12.0), t.text_dim);
            let name_room = rect.width() - 52.0 - rg.size().x - 16.0;
            let ng = ui.painter().layout(e.name.clone(), egui::FontId::proportional(15.0), t.text, name_room.max(40.0));
            ui.painter().with_clip_rect(rect).galley(egui::pos2(rect.left() + 44.0, rect.center().y - ng.size().y / 2.0), ng, t.text);
            ui.painter().galley(egui::pos2(rect.right() - rg.size().x - 10.0, rect.center().y - rg.size().y / 2.0), rg, t.text_dim);
            if resp.clicked() {
                if e.dir {
                    b.go(path);
                    break;
                }
                match &b.request {
                    PickRequest::Open(_) => out = Outcome::Answer(vec![path]),
                    PickRequest::OpenMany => {
                        if ticked {
                            b.ticked.retain(|p| *p != path);
                        } else {
                            b.ticked.push(path);
                        }
                    }
                    PickRequest::Save(_) => {
                        b.name = e.name.clone();
                        b.confirm_replace = false;
                    }
                    PickRequest::Folder => {}
                }
            }
        }
    });
    // Footer.
    ui.add_space(4.0);
    if let PickRequest::Save(_) = b.request {
        ui.horizontal(|ui| {
            ui.label(tl!("Name:"));
            let w = (ui.available_width()).max(80.0);
            if ui.add_sized(vec2(w, 36.0), egui::TextEdit::singleline(&mut b.name)).changed() {
                b.confirm_replace = false;
            }
        });
    }
    let filters: Vec<&'static str> = b.filters().iter().map(|(label, _)| *label).collect();
    if filters.len() > 1 {
        let lang = crate::i18n::current();
        let mut chosen = b.filter;
        egui::ComboBox::from_id_salt("file-browser-filter")
            .width(ui.available_width().min(360.0))
            .selected_text(filters.get(chosen).map(|l| crate::i18n::tr_ctx(lang, "file-filter", l)).unwrap_or(""))
            .show_ui(ui, |ui| {
                for (i, l) in filters.iter().enumerate() {
                    ui.selectable_value(&mut chosen, i, crate::i18n::tr_ctx(lang, "file-filter", l));
                }
            });
        if chosen != b.filter {
            b.choose_filter(chosen);
        }
    }
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match &b.request {
                PickRequest::Save(_) => {
                    let path = b.save_path();
                    let label = if b.confirm_replace { tl!("Replace") } else { tl!("Save") };
                    if big_button(ui, label, true, path.is_some()).clicked()
                        && let Some(path) = path
                    {
                        if path.is_dir() {
                            b.go(path);
                        } else if path.exists() && !b.confirm_replace {
                            b.confirm_replace = true;
                        } else {
                            out = Outcome::Answer(vec![path]);
                        }
                    }
                }
                PickRequest::OpenMany => {
                    let n = b.ticked.len();
                    if big_button(ui, tl!("Place"), true, n > 0).clicked() {
                        out = Outcome::Answer(b.ticked.clone());
                    }
                }
                PickRequest::Folder => {
                    if big_button(ui, tl!("Select"), true, b.error.is_none()).clicked() {
                        out = Outcome::Answer(vec![b.dir.clone()]);
                    }
                }
                PickRequest::Open(_) => {}
            }
            if big_button(ui, tl!("Cancel"), false, true).clicked() {
                out = Outcome::Cancel;
            }
        });
    });
    out
}

/// Room the footer under the list takes.
fn footer_height(b: &Browsing) -> f32 {
    let mut h = ROW + 16.0;
    if matches!(b.request, PickRequest::Save(_)) {
        h += 44.0;
    }
    if b.filters().len() > 1 {
        h += 32.0;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vc-file-browser-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const SVG: (&str, &[&str]) = ("SVG", &["svg"]);
    const PDF: (&str, &[&str]) = ("PDF", &["pdf"]);

    fn browsing(request: PickRequest, dir: &Path) -> (FileBrowser, std::sync::mpsc::Receiver<PickAnswer>) {
        let mut fb = FileBrowser::new(vec![Place { label: "Here".into(), path: dir.to_path_buf() }]);
        let (tx, rx) = channel();
        fb.start(request, tx);
        (fb, rx)
    }

    #[test]
    fn folders_come_first_and_hidden_files_stay_out() {
        let d = temp_dir("list");
        std::fs::write(d.join("b.svg"), b"x").unwrap();
        std::fs::write(d.join("A.svg"), b"x").unwrap();
        std::fs::write(d.join(".hidden"), b"x").unwrap();
        std::fs::create_dir(d.join("zeta")).unwrap();
        let names: Vec<String> = list(&d).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["zeta", "A.svg", "b.svg"]);
        assert!(list(&d.join("missing")).is_err());
    }

    #[test]
    fn starts_in_the_pick_folder_else_the_last_else_the_first_place() {
        let d = temp_dir("start");
        let sub = d.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let (mut fb, _rx) = browsing(PickRequest::Folder, &d);
        assert_eq!(fb.open.as_ref().unwrap().dir, d);
        fb.answer(vec![sub.clone()]);
        let (tx, _rx2) = channel();
        fb.last_dir = Some(sub.clone());
        fb.start(PickRequest::Folder, tx);
        assert_eq!(fb.open.as_ref().unwrap().dir, sub);
        let (tx, _rx3) = channel();
        let pick = FilePick { folder: Some(d.to_string_lossy().to_string()), ..Default::default() };
        fb.start(PickRequest::Open(pick), tx);
        assert_eq!(fb.open.as_ref().unwrap().dir, d);
    }

    #[test]
    fn the_type_chosen_filters_files_and_names_the_save() {
        let d = temp_dir("filter");
        let pick = FilePick { name: "Untitled-1.svg".into(), filters: vec![SVG, PDF], ..Default::default() };
        let (mut fb, _rx) = browsing(PickRequest::Save(pick), &d);
        let b = fb.open.as_mut().unwrap();
        assert!(b.shows("a.SVG") && !b.shows("a.pdf"));
        b.choose_filter(1);
        assert_eq!(b.name, "Untitled-1.pdf");
        assert!(b.shows("a.pdf"));
        b.name = "poster".into();
        assert_eq!(b.save_path(), Some(d.join("poster.pdf")));
        b.name = "  ".into();
        assert_eq!(b.save_path(), None);
        b.name = "a/b".into();
        assert_eq!(b.save_path(), None);
    }

    #[test]
    fn cancel_answers_nothing_and_answer_sends_the_paths() {
        let d = temp_dir("answer");
        let (mut fb, rx) = browsing(PickRequest::OpenMany, &d);
        fb.cancel();
        assert_eq!(rx.try_recv().unwrap(), Vec::<String>::new());
        assert!(!fb.is_open());
        let (tx, rx) = channel();
        fb.start(PickRequest::OpenMany, tx);
        fb.answer(vec![d.join("x.svg")]);
        assert_eq!(rx.try_recv().unwrap(), vec![d.join("x.svg").to_string_lossy().to_string()]);
        assert_eq!(fb.last_dir, Some(d));
    }

    /// Drawn headless, Open answers with the file tapped, and the starter hands the answer to the
    /// app's picks.
    #[test]
    fn a_tap_on_a_file_opens_it() {
        let d = temp_dir("tap");
        std::fs::write(d.join("art.svg"), b"<svg/>").unwrap();
        let fb = Rc::new(RefCell::new(FileBrowser::new(vec![Place { label: "Here".into(), path: d.clone() }])));
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut start = FileBrowser::starter(&fb, ctx.clone());
        let rx = start(PickRequest::Open(FilePick { filters: vec![SVG], ..Default::default() })).unwrap();
        let size = vec2(400.0, 800.0);
        // Draw a frame with `events`: where each text was drawn.
        let frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| fb.borrow_mut().show(ui.ctx()));
            out.textures_delta.clear();
            let mut texts = vec![];
            for c in &out.shapes {
                if let egui::Shape::Text(t) = &c.shape {
                    texts.push((t.galley.text().to_string(), t.visual_bounding_rect()));
                }
            }
            texts
        };
        frame(vec![]);
        let texts = frame(vec![]);
        let at = texts.iter().find(|(s, _)| s == "art.svg").map(|(_, r)| r.center()).unwrap();
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
        frame(vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        let answered = rx.try_recv().ok();
        assert_eq!(answered, Some(vec![d.join("art.svg").to_string_lossy().to_string()]));
        assert!(!fb.borrow().is_open());
    }
}
