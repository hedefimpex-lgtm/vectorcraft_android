//! View › Touch Layout (`view.touchLayout`): the window laid out for a phone or a tablet held in
//! the hand. Desktop chrome sized for a mouse (the menu bar, the control bar, the status and hint
//! bars, the docked Tools panel and the panel dock) gives way to two bars of fingertip-sized
//! buttons:
//!
//! - along the top: the menus (all of them, one tap deep), the active document's name, Open, Save,
//!   Undo, Redo, Fit Artboard, the selection's commands (the canvas's context menu, also a long
//!   press on the art) and Panels, which shows or hides the dock;
//! - along the bottom: the tools, one button per tool group, scrolling sideways; a long press on a
//!   group lists all of its tools. The status line sits above them while it has something to say.
//!
//! Pinching zooms, two fingers pan, and two- and three-finger taps undo and redo, on the canvas as
//! in the normal layout (`canvas`, `touch`).

use egui::{CornerRadius, Sense, Stroke, Ui, vec2};
use serde_json::json;
use vectorcraft_tools::tool_info;

use crate::theme::Tokens;
use crate::{VectorcraftApp, icons, menus, toolbar, widgets};

/// Side of a top-bar button.
const BUTTON: f32 = 44.0;
/// Height of the top bar.
const TOP: f32 = 52.0;
/// Side of a tool button in the bottom strip.
const TOOL: f32 = 52.0;

/// The top bar.
pub(crate) fn top_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.app_bar).inner_margin(egui::Margin::symmetric(4, 0)).stroke(Stroke::new(1.0, t.border));
    egui::Panel::top("touch_top_bar").exact_size(TOP).frame(frame).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut clicked = None;
            let menu = widgets::icon_button(ui, "menu", tl!("Menu"), false, BUTTON);
            egui::Popup::menu(&menu).show(|ui| {
                touch_menu_style(ui);
                menus::touch_menus(app, ui, &mut clicked);
            });
            // The buttons on the right, then the name in the room left between.
            let rest = ui.available_rect_before_wrap();
            let mut right = ui.new_child(egui::UiBuilder::new().max_rect(rest).layout(egui::Layout::right_to_left(egui::Align::Center)));
            let ui_r = &mut right;
            ui_r.spacing_mut().item_spacing.x = 2.0;
            if widgets::icon_button(ui_r, "layers", tl!("Panels"), app.ui.dock, BUTTON).clicked() {
                app.ui.dock = !app.ui.dock;
            }
            let more = widgets::icon_button(ui_r, "ellipsis", tl!("More"), false, BUTTON);
            egui::Popup::menu(&more).show(|ui| {
                touch_menu_style(ui);
                menus::context_menu_body(app, ui, &mut clicked);
            });
            // Right to left, most needed first: a narrow phone drops the last ones.
            for (icon, label, id) in [
                ("redo-2", "Redo", "edit.redo"),
                ("undo-2", "Undo", "edit.undo"),
                ("save", "Save", "file.save"),
                ("folder-open", "Open…", "file.open"),
                ("maximize-2", "Fit Artboard in Window", "view.fitArtboard"),
            ] {
                if ui_r.available_width() < BUTTON + 4.0 {
                    break;
                }
                let on = menus::enabled(app, id);
                if widgets::icon_button_enabled(ui_r, icon, crate::i18n::t(label), false, on, BUTTON).clicked() {
                    clicked = Some((id.to_string(), json!({})));
                }
            }
            let name_room =
                egui::Rect::from_min_max(egui::pos2(ui.cursor().min.x + 6.0, rest.top()), egui::pos2(ui_r.min_rect().left() - 6.0, rest.bottom()));
            if name_room.width() > 24.0
                && let Some(d) = app.session.active()
            {
                let title = format!("{}{}", d.title(), if d.is_dirty() { " •" } else { "" });
                let g = ui.painter().layout_no_wrap(title, egui::FontId::proportional(15.0), t.text_strong);
                let at = egui::pos2(name_room.left(), name_room.center().y - g.size().y / 2.0);
                ui.painter().with_clip_rect(name_room).galley(at, g, t.text_strong);
            }
            if let Some((id, p)) = clicked {
                menus::invoke(app, &id, p);
            }
        });
    });
}

/// Menus opened from the top bar: rows a finger can hit.
fn touch_menu_style(ui: &mut Ui) {
    let s = ui.spacing_mut();
    s.button_padding = vec2(12.0, 10.0);
    s.interact_size.y = 40.0;
    s.item_spacing.y = 4.0;
    s.menu_width = s.menu_width.max(240.0);
}

/// The tools along the bottom, and the status line above them while it has something to say.
pub(crate) fn tool_strip(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let status = !app.ui.status.is_empty();
    let h = TOOL + 12.0 + if status { 22.0 } else { 0.0 };
    let frame = egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(4, 4)).stroke(Stroke::new(1.0, t.border));
    egui::Panel::bottom("touch_tool_strip").exact_size(h).frame(frame).show(ui, |ui| {
        if status {
            let text = app.ui.status.clone();
            ui.add_sized(vec2(ui.available_width(), 18.0), egui::Label::new(egui::RichText::new(text).size(13.0).color(t.text_dim)).truncate());
        }
        let active = app.session.tool_id().to_string();
        egui::ScrollArea::horizontal().id_salt("touch-tools").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (_, slot) in toolbar::slots(app) {
                    let Some(first) = slot.first() else { continue };
                    let is_active = slot.iter().any(|s| *s == active);
                    let shown_id =
                        if is_active { active.clone() } else { app.ui.slot_tool.get(*first).cloned().unwrap_or_else(|| first.to_string()) };
                    let Some(shown) = tool_info(&shown_id).or_else(|| tool_info(first)) else { continue };
                    let (rect, resp) = ui.allocate_exact_size(vec2(TOOL, TOOL), Sense::click());
                    let bg = if is_active {
                        t.tool_active
                    } else if resp.hovered() {
                        t.hover
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect.shrink(1.0), CornerRadius::same(8), bg);
                    let ir = egui::Rect::from_center_size(rect.center(), vec2(26.0, 26.0));
                    icons::paint(ui, icons::tool_icon(shown.icon), ir, if is_active { t.text_strong } else { t.icon });
                    if slot.len() > 1 {
                        let c = rect.right_bottom() - vec2(7.0, 7.0);
                        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-6.0, 0.0), c + vec2(0.0, -6.0)], t.icon, Stroke::NONE));
                    }
                    // A long press (or a right click) lists the group's tools; a tap picks the one shown.
                    let open = if resp.secondary_clicked() && slot.len() > 1 {
                        Some(egui::SetOpenCommand::Bool(true))
                    } else if resp.clicked() {
                        app.select_tool(shown.id);
                        Some(egui::SetOpenCommand::Bool(false))
                    } else {
                        None
                    };
                    let resp = resp.on_hover_text(toolbar::tip(shown));
                    egui::Popup::menu(&resp).open_memory(open).align(egui::RectAlign::TOP_START).show(|ui| {
                        touch_menu_style(ui);
                        for id in &slot {
                            let Some(info) = tool_info(id) else { continue };
                            let (r, row) = ui.allocate_exact_size(vec2(240.0, 44.0), Sense::click());
                            if *id == active {
                                ui.painter().rect_filled(r, CornerRadius::same(6), t.tool_active);
                            } else if row.hovered() {
                                ui.painter().rect_filled(r, CornerRadius::same(6), t.hover);
                            }
                            icons::paint(
                                ui,
                                icons::tool_icon(info.icon),
                                egui::Rect::from_center_size(r.left_center() + vec2(22.0, 0.0), vec2(22.0, 22.0)),
                                t.icon,
                            );
                            ui.painter().text(
                                r.left_center() + vec2(44.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                tl!(info.label),
                                egui::FontId::proportional(15.0),
                                t.text,
                            );
                            if row.clicked() {
                                app.select_tool(info.id);
                                toolbar::remember(app, info.id);
                                ui.close();
                            }
                        }
                    });
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{Services, VectorcraftApp};

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(vectorcraft_engine::Session::new(), Services::default())
    }

    /// Draw a frame of `app` in a phone-sized window: the texts drawn and where.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        let size = egui::vec2(400.0, 800.0);
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        let mut texts = vec![];
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape {
                texts.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
        }
        texts
    }

    #[test]
    fn the_command_toggles_it_and_hides_the_panels() {
        let mut app = app();
        assert!(!app.ui.touch_layout && app.ui.dock);
        assert_eq!(app.run("view.touchLayout", json!({})).unwrap(), json!(true));
        assert!(app.ui.touch_layout && !app.ui.dock && !app.ui.control_bar);
        assert_eq!(crate::menus::checked(&app, "view.touchLayout", &json!({})), Some(true));
        assert_eq!(app.run("view.touchLayout", json!({"on": false})).unwrap(), json!(false));
        assert!(!app.ui.touch_layout);
        assert!(app.run("view.touchLayout", json!({"on": 1})).is_err());
    }

    /// The touch layout draws no menu bar (its menus are under the Menu button) and shows the
    /// active document's name; the normal layout draws the bar.
    #[test]
    fn the_touch_layout_replaces_the_menu_bar() {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(s, _)| s == "File"), "the normal layout has the menu bar");
        app.run("view.touchLayout", json!({"on": true})).unwrap();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(!texts.iter().any(|(s, _)| s == "File"), "no menu bar: {texts:?}");
        let title = app.session.active().unwrap().title();
        assert!(texts.iter().any(|(s, _)| s.starts_with(&title)), "the document's name: {texts:?}");
    }

    /// A tap on a tool in the bottom strip picks it.
    #[test]
    fn a_tap_on_the_strip_picks_the_tool() {
        let mut app = app();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("view.touchLayout", json!({"on": true})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        // The strip's tool buttons sit along the bottom: tap each until one picks another tool.
        let mut picked = None;
        let mut x = 30.0;
        while picked.is_none() && x < 390.0 {
            let p = egui::pos2(x, 800.0 - 34.0);
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
            );
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }],
            );
            let now = app.session.tool_id().to_string();
            if now != "selection" {
                picked = Some(now);
            }
            x += 56.0;
        }
        assert!(picked.is_some(), "a tap on the strip picks a tool");
    }
}
