//! The settings panel: every setting as a labelled control, over the page it restyles.
//!
//! Structure and wording are [`prefs`](crate::reader::prefs); this file draws them and decides
//! nothing else. It walks `prefs::fields()` in order, opens a heading whenever the group
//! changes, and writes back through `prefs::set`, so a setting added to the model appears here
//! with no change to this file.
//!
//! # Why it floats, and where
//!
//! Same frame as [`pageinfo_ui`](super::pageinfo_ui): `chrome_bg`, a `guide` stroke, the
//! radius, a `label_job` title with a `close` opposite. A summoned surface, not a bar, for the
//! reason recorded there — a bar is shown without asking and this is asked for.
//!
//! It anchors `RIGHT_TOP` rather than `RIGHT_BOTTOM`. `pageinfo_ui` rises from the italic `i`
//! because it is about the status strip; this one is about the whole reader and is summoned
//! from the menu bar in the opposite corner, so rising from the strip would point at the wrong
//! thing. Centring it, the way the help card sits, would be worse: the sliders here change the
//! typography of the column, and covering the column is covering the only feedback there is.
//!
//! # Why the sliders wear `-` and `+`
//!
//! `app::handle_keys` consumes `ArrowUp` and `ArrowDown` for scrolling before any widget sees
//! them, so a focused egui slider cannot be nudged with the arrow keys. Rather than narrow the
//! keymap — the `typing` guard is an allowlist of two widget ids, and widening it to "any
//! focused chrome widget" would risk `j`/`k` going dead after an ordinary click on a strip
//! button — each numeric field gets a step button either side. Those are reachable by Tab and
//! `Enter`, so the panel is operable from the keyboard, and scrolling the page while the panel
//! is open keeps working, which is what makes watching the text reflow useful.
//!
//! The glyphs are ASCII `-` and `+` on purpose. Every character this reader draws has to be in
//! one of the twelve faces `ui::fonts` embeds, and reaching for `\u{2212}` would reopen a
//! question ASCII closes.

use crate::reader::prefs::{self, Control, Field, FieldId, Group, Value};
use crate::reader::settings::Settings;
use crate::reader::ui::theme::{self, Palette};
use eframe::egui::{self, RichText, Ui};

/// What the panel did this frame, for `app` to apply after the borrow ends.
///
/// Zoom is the reason this is a value rather than a direct write. `app::persist` copies
/// `ctx.zoom_factor()` *into* the settings every frame, so assigning the field here would be
/// overwritten before it ever reached egui; the zoom row has to go through
/// `Context::set_zoom_factor`, and that is `app`'s call to make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Changed,
    ZoomSet(f32),
}

/// Draw the panel. `open` is `Chrome::settings_open`, so `Esc`, `,`, the menu item and the
/// window's own close all reach the same bit.
///
/// Returns every change made this frame, in order. `app` applies them and sets `settings_dirty`,
/// which the existing 1.5 s debounce and the `on_exit` flush already carry to disk.
pub fn panel(
    ctx: &egui::Context,
    pal: &Palette,
    settings: &mut Settings,
    strip_height: f32,
    open: &mut bool,
) -> Vec<Event> {
    let mut events = Vec::new();
    let opts = settings.read.clone();
    let font = theme::chrome_font(&opts);
    let fields = prefs::fields();

    egui::Area::new(egui::Id::new("hww-settings-panel"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal.chrome_bg)
                .stroke(egui::Stroke::new(1.0, pal.guide))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.style_mut().override_font_id = Some(font.clone());
                    ui.set_max_width(theme::snap(opts.base_size_pt * 24.0));

                    ui.horizontal(|ui| {
                        ui.label(theme::label_job("Settings", &opts, pal.dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("close").clicked() {
                                *open = false;
                            }
                        });
                    });
                    ui.add_space(theme::snap(opts.base_size_pt * 0.4));

                    // A panel taller than a short window has to scroll, or its foot — which is
                    // where `reset all` and the file path are — cannot be reached. The same
                    // ceiling `pageinfo_ui` takes, and for the same reason.
                    // From the whole window, not `content_rect`: this is drawn after the
                    // central panel, so the content rect has already shrunk past every docked
                    // surface and left the panel a third of the height it can actually use.
                    // Fourteen settings, each with a sentence, need every pixel there is.
                    let max_h = (ctx.content_rect().height() - strip_height - 76.0).max(200.0);
                    // Allocated at a definite height rather than left to take "available".
                    // An `Area`'s `Ui` gets its `max_rect` from the size the area measured on
                    // the *previous* frame, so a scroll area that sizes itself to the space on
                    // offer feeds its own answer back in and settles wherever it first landed:
                    // this panel stuck at 332 points of a 692-point window and showed two of its
                    // fourteen settings. Asking for the height outright breaks the loop.
                    let width = theme::snap(opts.base_size_pt * 24.0);
                    ui.allocate_ui(egui::vec2(width, max_h), |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(max_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let mut current: Option<Group> = None;
                                for field in &fields {
                                    if current != Some(field.group) {
                                        if current.is_some() {
                                            ui.add_space(theme::snap(opts.base_size_pt * 0.35));
                                            ui.separator();
                                            ui.add_space(theme::snap(opts.base_size_pt * 0.35));
                                        }
                                        group_heading(
                                            ui,
                                            pal,
                                            &opts,
                                            field.group,
                                            settings,
                                            &mut events,
                                        );
                                        current = Some(field.group);
                                    }
                                    row(ui, pal, &opts, field, settings, &mut events);
                                    ui.add_space(theme::snap(opts.base_size_pt * 0.45));
                                }

                                ui.add_space(theme::snap(opts.base_size_pt * 0.2));
                                ui.separator();
                                ui.add_space(theme::snap(opts.base_size_pt * 0.3));
                                ui.horizontal(|ui| {
                                    if ui.button("reset all").clicked() {
                                        prefs::reset_all(settings);
                                        events.push(Event::Changed);
                                        events.push(Event::ZoomSet(settings.zoom_factor));
                                    }
                                });
                                ui.add_space(theme::snap(opts.base_size_pt * 0.3));
                                // Nothing else in the reader has ever said where this lives, and
                                // hand-editing stays supported.
                                if let Some(path) = crate::reader::settings::settings_path() {
                                    ui.label(
                                        RichText::new(format!("Saved in {}", path.display()))
                                            .color(pal.dim)
                                            .font(font.clone()),
                                    );
                                }
                            });
                    });
                });
        });
    events
}

/// A group heading, its optional sentence, and the `reset` that puts the group back.
fn group_heading(
    ui: &mut Ui,
    pal: &Palette,
    opts: &crate::reader::opts::ReadOpts,
    group: Group,
    settings: &mut Settings,
    events: &mut Vec<Event>,
) {
    ui.horizontal(|ui| {
        ui.label(theme::label_job(group.title(), opts, pal.dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("reset")
                .on_hover_text("Put this group back to its defaults")
                .clicked()
            {
                prefs::reset_group(settings, group);
                events.push(Event::Changed);
                if group == Group::Page {
                    events.push(Event::ZoomSet(settings.zoom_factor));
                }
            }
        });
    });
    if let Some(note) = group.note() {
        ui.add_space(theme::snap(opts.base_size_pt * 0.15));
        wrapped(ui, pal.dim, opts, note);
    }
    ui.add_space(theme::snap(opts.base_size_pt * 0.3));
}

/// One setting: its label, the key that already does it, its control, and its sentence.
fn row(
    ui: &mut Ui,
    pal: &Palette,
    opts: &crate::reader::opts::ReadOpts,
    field: &Field,
    settings: &mut Settings,
    events: &mut Vec<Event>,
) {
    let font = theme::chrome_font(opts);
    ui.horizontal(|ui| {
        ui.label(RichText::new(field.label).color(pal.fg).font(font.clone()));
        if !field.keys.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(field.keys).color(pal.dim).font(font.clone()));
            });
        }
    });
    ui.add_space(theme::snap(opts.base_size_pt * 0.15));

    match &field.control {
        Control::Slider {
            min,
            max,
            step,
            fractional,
            unit,
        } => slider(
            ui,
            pal,
            opts,
            field,
            *min,
            *max,
            *step,
            *fractional,
            unit,
            settings,
            events,
        ),
        Control::Choice(options) => {
            let now = prefs::get(settings, field.id).index().unwrap_or(0);
            for (i, option) in options.iter().enumerate() {
                if ui
                    .radio(now == i, RichText::new(option.label).font(font.clone()))
                    .clicked()
                    && now != i
                {
                    prefs::set(settings, field.id, Value::Index(i));
                    events.push(Event::Changed);
                }
                if let Some(note) = option.note {
                    ui.indent(("opt", field.id, i), |ui| {
                        wrapped(ui, pal.dim, opts, note);
                    });
                }
            }
        }
        Control::Toggle => {
            let mut on = prefs::get(settings, field.id).boolean().unwrap_or(false);
            if ui.checkbox(&mut on, "").changed() {
                prefs::set(settings, field.id, Value::Bool(on));
                events.push(Event::Changed);
            }
        }
    }

    if let Some(note) = field.note {
        ui.add_space(theme::snap(opts.base_size_pt * 0.15));
        wrapped(ui, pal.dim, opts, note);
    }
}

/// A numeric field: `-`, the slider, `+`, then the value with its unit.
#[allow(clippy::too_many_arguments)]
fn slider(
    ui: &mut Ui,
    pal: &Palette,
    opts: &crate::reader::opts::ReadOpts,
    field: &Field,
    min: f32,
    max: f32,
    step: f32,
    fractional: bool,
    unit: &str,
    settings: &mut Settings,
    events: &mut Vec<Event>,
) {
    let font = theme::chrome_font(opts);
    let before = prefs::get(settings, field.id).num().unwrap_or(min);
    let mut value = before;

    ui.horizontal(|ui| {
        // The step buttons are the keyboard route; see the module doc.
        if ui.button("-").on_hover_text("Smaller").clicked() {
            value = (value - step).max(min);
        }
        ui.add(
            egui::Slider::new(&mut value, min..=max)
                .show_value(false)
                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
        );
        if ui.button("+").on_hover_text("Larger").clicked() {
            value = (value + step).min(max);
        }
        let shown = if fractional {
            format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        } else {
            format!("{}", value.round() as i64)
        };
        ui.label(
            RichText::new(if unit.is_empty() {
                shown
            } else {
                format!("{shown} {unit}")
            })
            .color(pal.dim)
            .font(font.clone()),
        );
    });

    if (value - before).abs() > f32::EPSILON {
        prefs::set(settings, field.id, Value::Num(value));
        events.push(Event::Changed);
        if field.id == FieldId::Zoom {
            events.push(Event::ZoomSet(settings.zoom_factor));
        }
    }
}

/// A note, wrapped inside the panel rather than widening it.
///
/// `chrome_font` by name, not `.small()`: `theme` maps `TextStyle::Small` to the page's family,
/// and one proportional line in a monospace panel is the bug `pageinfo_ui` already records.
fn wrapped(ui: &mut Ui, color: egui::Color32, opts: &crate::reader::opts::ReadOpts, text: &str) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    ui.label(
        RichText::new(text)
            .color(color)
            .font(theme::chrome_font(opts)),
    );
}
