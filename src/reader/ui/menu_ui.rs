//! The menu bar: the reader's command list, where a pointer can find it.
//!
//! [`menu`](crate::reader::menu) holds the table; this walks it. Radio submenus take their
//! option labels from [`prefs`](crate::reader::prefs), so the words a reader sees in
//! `View` ▸ `Theme` and in the settings panel are one set of strings.
//!
//! # The bar
//!
//! A top panel above the URL bar, `chrome_bg` with one `guide` edge through the shared
//! `panel_edge`, like every other docked surface. No radius: it spans the window, and an edge
//! has no corner to round. The popups it opens do get one, from `theme::apply`'s
//! `menu_corner_radius`, and they take `window_fill` and `window_stroke` from the palette
//! already — the link context menu has been dressing itself that way since before this existed,
//! so the bar needed no new colour work.
//!
//! Menu titles are plain words in `chrome_font`, not `theme::label_job`: a label job is
//! uppercase with tracking and belongs over a group of things, not on a control.
//!
//! # The submenu marker
//!
//! Set from [`menu::SUBMENU_MARKER`] rather than left to egui, whose default `⏵` (U+23F5) is in
//! none of the faces this binary carries and draws as a box. See the `menu` module doc.
//!
//! # Keyboard
//!
//! egui menus handle no arrow keys — `Escape` is the only key its popups read — so traversal
//! here is the ordinary Tab focus cycle and `Enter`. `F10` focuses the first title, which is
//! what [`Bar::focus_first`] is for: `MenuButton` takes `ui.next_auto_id()`, so the id cannot be
//! known before the bar is drawn and the response has to be handed back to `app` instead.

use crate::reader::menu::{self, Checked, Command, Item, Needs};
use crate::reader::prefs::{self, Control};
use crate::reader::settings::Settings;
use crate::reader::ui::theme::{self, Palette};
use eframe::egui::{self, RichText, Ui};

/// What the reader can currently do, so an item that would fire on nothing is greyed instead.
///
/// The status strip already does this for Back and Forward off `History::can_go_back`; this is
/// the same courtesy for the rest of the commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct Enable {
    pub page: bool,
    pub focused_link: bool,
    pub thread: bool,
    pub back: bool,
    pub forward: bool,
}

impl Enable {
    fn allows(self, needs: Needs) -> bool {
        match needs {
            Needs::Nothing => true,
            Needs::Page => self.page,
            Needs::FocusedLink => self.focused_link,
            Needs::Thread => self.thread,
            Needs::Back => self.back,
            Needs::Forward => self.forward,
        }
    }
}

/// The live state a checked item reflects.
#[derive(Debug, Clone, Copy, Default)]
pub struct Checks {
    pub link_addresses: bool,
    pub outline: bool,
    pub page_info: bool,
    pub menu_bar: bool,
}

impl Checks {
    fn is_on(self, what: Checked) -> bool {
        match what {
            Checked::LinkAddresses => self.link_addresses,
            Checked::Outline => self.outline,
            Checked::PageInfo => self.page_info,
            Checked::MenuBar => self.menu_bar,
        }
    }
}

/// What the bar drew and what was chosen.
pub struct Bar {
    /// The command a click asked for. One per frame, first wins, the way `RenderCtx::act` does:
    /// a frame in which two menu items both fire is a frame with a bug in it.
    pub command: Option<Command>,
    /// The first menu title's response, so `F10` has something to focus.
    pub focus_first: Option<egui::Response>,
    /// The bar's own rect, for `panel_edge`.
    pub rect: egui::Rect,
}

/// Draw the bar. Returns the chosen command, if any.
pub fn bar(ui: &mut Ui, pal: &Palette, settings: &Settings, enable: Enable, checks: Checks) -> Bar {
    let opts = &settings.read;
    let mut command: Option<Command> = None;
    let mut focus_first = None;
    let font = theme::chrome_font(opts);

    let panel = egui::Panel::top("hww-menubar")
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(pal.chrome_bg)
                .inner_margin(egui::Margin::symmetric(8, 3)),
        )
        .show(ui, |ui| {
            // Every word here is the reader's, so the family is set once rather than at each
            // label. `status_strip` does the same and for the same reason.
            ui.style_mut().override_font_id = Some(font.clone());
            egui::MenuBar::new().ui(ui, |ui| {
                for (i, m) in menu::bar().into_iter().enumerate() {
                    let button = egui::containers::menu::MenuButton::new(
                        RichText::new(m.title).color(pal.fg).font(font.clone()),
                    );
                    let (response, _) = button.ui(ui, |ui| {
                        ui.style_mut().override_font_id = Some(font.clone());
                        ui.set_min_width(theme::snap(opts.base_size_pt * 13.0));
                        for item in &m.items {
                            draw_item(ui, pal, settings, item, enable, checks, &mut command);
                        }
                    });
                    if i == 0 {
                        focus_first = Some(response);
                    }
                }
            });
        });

    Bar {
        command,
        focus_first,
        rect: panel.response.rect,
    }
}

fn draw_item(
    ui: &mut Ui,
    pal: &Palette,
    settings: &Settings,
    item: &Item,
    enable: Enable,
    checks: Checks,
    command: &mut Option<Command>,
) {
    let opts = &settings.read;
    let font = theme::chrome_font(opts);
    let mut choose = |c: Command| {
        if command.is_none() {
            *command = Some(c);
        }
    };

    match item {
        Item::Separator => {
            ui.separator();
        }
        Item::Run {
            label,
            keys,
            command: c,
            needs,
        } => {
            let mut button = egui::Button::new(RichText::new(*label).font(font.clone()));
            if !keys.is_empty() {
                button = button.shortcut_text(RichText::new(*keys).color(pal.dim).font(font));
            }
            if ui.add_enabled(enable.allows(*needs), button).clicked() {
                choose(*c);
                ui.close();
            }
        }
        Item::Check {
            label,
            keys,
            command: c,
            checked,
            needs,
        } => {
            // A check mark egui paints itself, rather than a character: `✓` is in the Plex
            // faces but the box is not, and a lone tick with nothing to sit in reads as a
            // bullet. `Checkbox` is drawn from `Shape::line`, so there is no glyph to be
            // missing.
            let mut on = checks.is_on(*checked);
            let widget = egui::Checkbox::new(&mut on, RichText::new(*label).font(font.clone()));
            let mut resp = ui.add_enabled(enable.allows(*needs), widget);
            if !keys.is_empty() {
                resp = resp.on_hover_text(*keys);
            }
            if resp.changed() {
                choose(*c);
                ui.close();
            }
        }
        Item::Radio { label, family } => {
            let field = family.field();
            let all = prefs::fields();
            let Some(f) = all.iter().find(|f| f.id == field) else {
                return;
            };
            let Control::Choice(options) = f.control else {
                return;
            };
            // Read straight off the settings, the same way the panel does, so the tick in
            // `View` and the radio in the panel cannot disagree about what is selected.
            let current = prefs::get(settings, field).index().unwrap_or(usize::MAX);
            let button = egui::containers::menu::SubMenuButton::from_button(
                egui::Button::new(RichText::new(*label).font(font.clone()))
                    .right_text(RichText::new(menu::SUBMENU_MARKER).color(pal.dim)),
            );
            button.ui(ui, |ui| {
                ui.style_mut().override_font_id = Some(theme::chrome_font(opts));
                for (i, option) in options.iter().enumerate() {
                    let mut resp = ui.radio(current == i, RichText::new(option.label));
                    if let Some(note) = option.note {
                        resp = resp.on_hover_text(note);
                    }
                    if resp.clicked() {
                        choose(family.command(i));
                        ui.close();
                    }
                }
            });
        }
    }
}
