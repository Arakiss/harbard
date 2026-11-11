//! "Profile" (`profile-dialog.tsx`): who the operator is for the agents —
//! a photo or a gallery bust, name, how to address them, time zone, working
//! hours, and the context every new session reads in its brief.

use super::shell_profile::{ProfileStore, refusal_text};
use super::stores::{ProfilePhoto, update_profile};
use super::widgets::{self, Kind, Size};
use crate::derive::profile::{
    CONTEXT_MAX, WEEKDAYS, context_count, context_fits, this_mac_zone, thousands, time_zones,
};
use crate::i18n::{I18n, Locale, t, tv};
use crate::model::profile::{ProfileHours, ProfileLimits, ProfileView};
use crate::theme::Theme;
use crate::ui::{self, control, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, img, px,
};
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use serde_json::{Value, json};

/// The dialog, its photo column, the photo, a gallery bust, a time field,
/// the context box and the time-zone menu.
const DIALOG_W: f32 = 860.;
const SIDE_W: f32 = 200.;
const PHOTO: f32 = 160.;
const THUMB: f32 = 44.;
const TIME_W: f32 = 96.;
const CONTEXT_H: f32 = 168.;
const ZONE_MENU_H: f32 = 320.;

/// The gallery: (id, label key, …) in the console's order (`lib/avatar-gallery.ts`).
pub const AVATARS: [(&str, &str); 24] = [
    ("operadora", "avatars.operatorHeadset"),
    ("zorro", "avatars.mechanicalFox"),
    ("piloto", "avatars.pilotGoggles"),
    ("gato", "avatars.captainCat"),
    ("inventora", "avatars.inventorMagnifier"),
    ("electricista", "avatars.electricianHardHat"),
    ("buho", "avatars.engineerOwl"),
    ("nutria", "avatars.otterWrench"),
    ("panda", "avatars.constructionPanda"),
    ("conejo", "avatars.welderRabbit"),
    ("pinguino", "avatars.technicianPenguin"),
    ("mapache", "avatars.handyRaccoon"),
    ("astronauta", "avatars.astronaut"),
    ("jardinera", "avatars.gardenerHat"),
    ("chef", "avatars.chefMoustache"),
    ("musica", "avatars.musicianAfro"),
    ("bombera", "avatars.firefighter"),
    ("exploradora", "avatars.explorer"),
    ("lobo", "avatars.wolfScarf"),
    ("erizo", "avatars.hedgehogGlasses"),
    ("rana", "avatars.frogRainHat"),
    ("koala", "avatars.koala"),
    ("dragon", "avatars.littleDragon"),
    ("yeti", "avatars.yetiHat"),
];

/// The flat backdrop every bust is drawn on.
const AVATAR_BACKDROP: u32 = 0xf4efe6;

/// A time zone in the select: the value serve stores and its label.
#[derive(Clone)]
pub struct Zone {
    value: String,
    label: SharedString,
}

impl SearchableListItem for Zone {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

/// Keep an input within `max` characters, as `maxLength` did.
fn cap(state: &Entity<InputState>, max: usize, window: &mut Window, cx: &mut App) {
    let v = state.read(cx).value().to_string();
    if v.chars().count() > max {
        let cut: String = v.chars().take(max).collect();
        state.update(cx, |s, cx| s.set_value(cut, window, cx));
    }
}

pub struct ProfileDialog {
    name: Entity<InputState>,
    address: Entity<InputState>,
    zone: Entity<SelectState<SearchableVec<Zone>>>,
    zone_value: String,
    has_hours: bool,
    start: Entity<InputState>,
    end: Entity<InputState>,
    days: Vec<u8>,
    context: Entity<TextareaState>,
    picked: Option<&'static str>,
    status: Option<SharedString>,
    error: Option<SharedString>,
    saving: bool,
    _subs: Vec<Subscription>,
}

/// serve's limits; the console's numbers until serve has answered.
fn limits_of(view: Option<&ProfileView>) -> ProfileLimits {
    let l = view.map(|v| v.limits.clone()).unwrap_or_default();
    let or = |v: u64, d: u64| if v == 0 { d } else { v };
    ProfileLimits {
        context: or(l.context, 1500),
        name: or(l.name, 60),
        address: or(l.address, 120),
        status_text: or(l.status_text, 100),
        photo_bytes: or(l.photo_bytes, 2 * 1024 * 1024),
    }
}

/// Working hours before any are set: 9:00 to 18:00, Monday to Friday.
fn default_hours() -> ProfileHours {
    ProfileHours {
        start: "09:00".into(),
        end: "18:00".into(),
        days: vec![1, 2, 3, 4, 5],
    }
}

impl ProfileDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe(&ProfilePhoto::get(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let store = ProfileStore::get(cx).read(cx);
        let stored = store
            .view
            .as_ref()
            .map(|v| v.profile.clone())
            .unwrap_or_default();
        let limits = limits_of(store.view.as_ref());
        let operator = crate::desk::Desk::get(cx)
            .read(cx)
            .view
            .as_ref()
            .map(|v| v.operator.clone())
            .unwrap_or_default();

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(operator.clone())
                .default_value(stored.name.clone().unwrap_or_default())
        });
        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t(cx, "profile.addressPlaceholder"))
                .default_value(stored.address.clone().unwrap_or_default())
        });
        let here = this_mac_zone();
        let mut zones = vec![Zone {
            value: String::new(),
            label: t(cx, "profile.timeZoneNotSet"),
        }];
        zones.extend(time_zones(&here).into_iter().map(|z| Zone {
            label: if z == here {
                tv(cx, "profile.timeZoneThisMac", &[("zone", &z)])
            } else {
                z.clone().into()
            },
            value: z,
        }));
        let zone_value = stored.time_zone.clone().unwrap_or_default();
        let selected = zones
            .iter()
            .position(|z| z.value == zone_value)
            .unwrap_or(0);
        let zone = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(zones),
                Some(IndexPath::new(selected)),
                window,
                cx,
            )
            .searchable(true)
        });
        let hours = stored.hours.clone().unwrap_or_else(default_hours);
        let start = cx.new(|cx| InputState::new(window, cx).default_value(hours.start.clone()));
        let end = cx.new(|cx| InputState::new(window, cx).default_value(hours.end.clone()));
        let context = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder(t(cx, "profile.contextPlaceholder"))
                .default_value(stored.context.clone().unwrap_or_default())
        });
        let (name_max, address_max) = (limits.name as usize, limits.address as usize);
        let subs = vec![
            cx.subscribe_in(&name, window, move |_, s, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    cap(s, name_max, window, cx);
                }
            }),
            cx.subscribe_in(
                &address,
                window,
                move |_, s, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::Change) {
                        cap(s, address_max, window, cx);
                    }
                },
            ),
            cx.subscribe_in(&context, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &zone,
                window,
                |this, _, ev: &SelectEvent<SearchableVec<Zone>>, _, cx| {
                    let SelectEvent::Confirm(v) = ev;
                    this.zone_value = v.clone().unwrap_or_default();
                    cx.notify();
                },
            ),
        ];
        ProfileDialog {
            name,
            address,
            zone,
            zone_value,
            has_hours: stored.hours.is_some(),
            start,
            end,
            days: hours.days,
            context,
            picked: None,
            status: None,
            error: None,
            saving: false,
            _subs: subs,
        }
    }

    fn pick_photo(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let limit = ProfileStore::get(cx)
            .read(cx)
            .view
            .as_ref()
            .map_or(2 * 1024 * 1024, |v| limits_of(Some(v)).photo_bytes);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let mime = match ext.as_str() {
                "png" => Some("image/png"),
                "jpg" | "jpeg" => Some("image/jpeg"),
                "webp" => Some("image/webp"),
                _ => None,
            };
            let bytes = cx
                .background_spawn(async move { std::fs::read(&path) })
                .await;
            let upload = this.update(cx, |this, cx| {
                this.error = None;
                let Some(mime) = mime else {
                    this.error = Some(t(cx, "profile.photoWrongType"));
                    cx.notify();
                    return None;
                };
                let Ok(bytes) = bytes else { return None };
                if bytes.len() as u64 > limit {
                    this.error = Some(t(cx, "profile.photoTooLarge"));
                    cx.notify();
                    return None;
                }
                Some(ProfilePhoto::upload(bytes, mime, cx))
            });
            let Ok(Some(upload)) = upload else { return };
            let out = upload.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(_) => this.status = Some(t(cx, "profile.photoSaved")),
                    Err(e) => this.error = Some(refusal_text(&e, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_photo(&mut self, cx: &mut Context<Self>) {
        let task = update_profile(json!({ "photo": null }), cx);
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(_) => this.status = Some(t(cx, "profile.photoRemoved")),
                    Err(e) => this.error = Some(refusal_text(&e, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_avatar(&mut self, id: &'static str, label: SharedString, cx: &mut Context<Self>) {
        self.picked = Some(id);
        self.error = None;
        let Some(bytes) = crate::assets::Assets::local(&format!("avatars/{id}-512.webp")) else {
            return;
        };
        let task = ProfilePhoto::upload(bytes.to_vec(), "image/webp", cx);
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(_) => {
                        this.status = Some(tv(cx, "profile.avatarChosen", &[("label", &label)]))
                    }
                    Err(e) => this.error = Some(refusal_text(&e, cx)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_hours && self.days.is_empty() {
            self.error = Some(t(cx, "profile.pickWorkDay"));
            cx.notify();
            return;
        }
        let trimmed = |s: SharedString| -> Value {
            let s = s.trim().to_string();
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(s)
            }
        };
        let hours = if self.has_hours {
            json!({
                "start": self.start.read(cx).value().trim(),
                "end": self.end.read(cx).value().trim(),
                "days": self.days,
            })
        } else {
            Value::Null
        };
        let patch = json!({
            "name": trimmed(self.name.read(cx).value()),
            "address": trimmed(self.address.read(cx).value()),
            "time_zone": if self.zone_value.is_empty() { Value::Null } else { Value::String(self.zone_value.clone()) },
            "hours": hours,
            "context": trimmed(self.context.read(cx).value()),
        });
        self.saving = true;
        self.error = None;
        cx.notify();
        let task = update_profile(patch, cx);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(_) => {
                        let _ = handle.update(cx, |_, window, cx| widgets::close_top(window, cx));
                    }
                    Err(e) => {
                        this.error = Some(refusal_text(&e, cx));
                        this.saving = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_side(&self, loaded: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let photo = ProfilePhoto::get(cx).read(cx).image.clone();
        let initials = crate::desk::Desk::get(cx)
            .read(cx)
            .view
            .as_ref()
            .map(|v| v.operator_initials.clone())
            .unwrap_or_default();
        let avatar = match photo.clone() {
            Some(image) => img(image)
                .size(px(PHOTO))
                .rounded(px(radius::LG))
                .into_any_element(),
            None => ui::avatar::operator_avatar(&initials, PHOTO, cx),
        };
        // The picked bust wears a neutral ring: accent is never a border (C2).
        let picked_ring = theme.c("ink-2");
        let edge = theme.c("line-hover");
        let line = theme.c("line");
        let small = |s: SharedString| ui::text(s, size::SMALL, weight::REGULAR, theme.c("ink-3"));
        div()
            .w(px(SIDE_W))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(div().mb(px(space::XS)).child(avatar))
            .child(widgets::button(
                "upload-photo",
                t(cx, "profile.uploadPhoto"),
                Kind::Plain,
                Size::Normal,
                Some("image-up"),
                !loaded,
                theme,
                cx.listener(|this, _, _, cx| this.pick_photo(cx)),
            ))
            .when(photo.is_some(), |d| {
                d.child(widgets::button(
                    "remove-photo",
                    t(cx, "profile.removePhoto"),
                    Kind::Quiet,
                    Size::Normal,
                    Some("trash"),
                    false,
                    theme,
                    cx.listener(|this, _, _, cx| this.remove_photo(cx)),
                ))
            })
            .child(small(t(cx, "profile.photoHint")))
            .child(small(t(cx, "profile.pickFromGallery")).mt(px(space::MD)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_y(px(space::MD))
                    .children(AVATARS.iter().map(|(id, key)| {
                        let id: &'static str = id;
                        let label = t(cx, key);
                        let l2 = label.clone();
                        let picked = self.picked == Some(id);
                        let el = div()
                            .id(SharedString::from(format!("avatar-{id}")))
                            .size(px(THUMB))
                            .rounded(px(radius::MD))
                            .overflow_hidden()
                            // The busts are drawn for this backdrop in every theme.
                            .bg(gpui::rgb(AVATAR_BACKDROP))
                            .border_1()
                            .border_color(if picked { picked_ring } else { line })
                            .when(picked, |d| {
                                d.shadow(vec![widgets::ring_shadow(
                                    picked_ring,
                                    space::HAIR,
                                    false,
                                )])
                            });
                        let el = if loaded {
                            widgets::focusable(el, theme, false)
                                .cursor_pointer()
                                .hover(move |s| s.border_color(edge))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pick_avatar(id, l2.clone(), cx)
                                }))
                        } else {
                            el.opacity(0.5)
                        };
                        el.tooltip(widgets::tip(label, None)).child(
                            img(SharedString::from(format!("avatars/{id}-192.webp"))).size_full(),
                        )
                    })),
            )
            .into_any_element()
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    ProfileStore::get(cx).update(cx, |p, cx| p.refresh(cx));
    let view = cx.new(|cx| ProfileDialog::new(window, cx));
    let first = view.read(cx).name.clone();
    widgets::open_modal(
        view.into(),
        DIALOG_W,
        None,
        widgets::Place::Center,
        None,
        window,
        cx,
    );
    widgets::autofocus(first, window, cx);
}

/// `.hb-field-block`: a label over its control.
fn field(label: SharedString, control: impl IntoElement, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(space::SM))
        .child(widgets::label(label, theme))
        .child(control)
}

impl Render for ProfileDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let loaded = ProfileStore::get(cx).read(cx).view.is_some();
        let spanish = cx.global::<I18n>().locale == Locale::Es;
        let max_h = f32::from(window.viewport_size().height) - 2. * space::SECTION;
        let context = self.context.read(cx).value().to_string();
        let count = context_count(&context);
        let over = count > CONTEXT_MAX;
        let heavy = !over && !context_fits(&context);
        let status: Option<AnyElement> = if let Some(e) = &self.error {
            Some(widgets::error(e.clone(), &theme).into_any_element())
        } else if over {
            let n = thousands(count - CONTEXT_MAX, spanish);
            Some(
                widgets::error(tv(cx, "profile.charactersOver", &[("count", &n)]), &theme)
                    .into_any_element(),
            )
        } else if heavy {
            Some(widgets::error(t(cx, "profile.contextTooHeavy"), &theme).into_any_element())
        } else {
            self.status.clone().map(|s| s.into_any_element())
        };

        let small = |s: SharedString| ui::text(s, size::SMALL, weight::REGULAR, theme.c("ink-3"));
        let has_hours = self.has_hours;
        let hours = div()
            .flex()
            .flex_col()
            .child(widgets::label(t(cx, "profile.workingHours"), &theme))
            .child(widgets::check_row(
                "hours-toggle",
                has_hours,
                t(cx, "profile.tellAgentsWhenIWork"),
                false,
                &theme,
                cx.listener(|this, _, _, cx| {
                    this.has_hours = !this.has_hours;
                    cx.notify();
                }),
            ))
            .when(has_hours, |d| {
                d.child(
                    div()
                        .mt(px(space::XS))
                        .flex()
                        .gap(px(space::XL))
                        .text_color(theme.c("ink-2"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::MD))
                                .child(t(cx, "profile.hoursFrom"))
                                .child(
                                    div()
                                        .w(px(TIME_W))
                                        .child(widgets::text_input(&self.start, &theme)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::MD))
                                .child(t(cx, "profile.hoursTo"))
                                .child(
                                    div()
                                        .w(px(TIME_W))
                                        .child(widgets::text_input(&self.end, &theme)),
                                ),
                        ),
                )
                .child(div().mt(px(space::MD)).flex().gap(px(space::SM)).children(
                    WEEKDAYS.iter().map(|(day, short, long)| {
                        let day = *day;
                        let on = self.days.contains(&day);
                        let long = t(cx, long);
                        let edge = theme.c("line-hover");
                        // A day is a small toggle: a control, not a pill (R2).
                        widgets::focusable(
                            div().id(SharedString::from(format!("weekday-{day}"))),
                            &theme,
                            false,
                        )
                        .size(px(control::MD))
                        .rounded(px(radius::BASE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(theme.c("line-strong"))
                        .cursor_pointer()
                        .hover(move |s| s.border_color(edge))
                        .map(|d| {
                            if on {
                                d.bg(theme.c("selected"))
                                    .text_color(theme.c("ink"))
                                    .font_weight(weight::STRONG)
                            } else {
                                d.bg(theme.c("surface")).text_color(theme.c("ink-3"))
                            }
                        })
                        .tooltip(widgets::tip(long, None))
                        .child(t(cx, short))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(i) = this.days.iter().position(|d| *d == day) {
                                this.days.remove(i);
                            } else {
                                this.days.push(day);
                                this.days.sort();
                            }
                            cx.notify();
                        }))
                    }),
                ))
            });

        let fields = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .child(field(
                t(cx, "profile.name"),
                widgets::text_input(&self.name, &theme),
                &theme,
            ))
            .child(
                field(
                    t(cx, "profile.howToAddress"),
                    widgets::text_input(&self.address, &theme),
                    &theme,
                )
                .child(small(t(cx, "profile.addressHint"))),
            )
            .child(field(
                t(cx, "profile.timeZone"),
                Select::new(&self.zone)
                    .h(px(control::MD))
                    .rounded(px(radius::BASE))
                    .border_color(theme.c("line-strong"))
                    .bg(theme.c("surface"))
                    .text_color(theme.c("ink"))
                    .menu_max_h(px(ZONE_MENU_H)),
                &theme,
            ))
            .child(hours)
            .child(
                field(
                    t(cx, "profile.contextForAgents"),
                    Textarea::new(&self.context)
                        .h(px(CONTEXT_H))
                        .rounded(px(radius::BASE))
                        .border_color(if over || heavy {
                            theme.c("red")
                        } else {
                            theme.c("line-strong")
                        })
                        .bg(theme.c("surface"))
                        .text_color(theme.c("ink")),
                    &theme,
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .gap(px(space::LG))
                        .text_size(px(size::SMALL))
                        .line_height(px(leading::SMALL))
                        .child(small(t(cx, "profile.contextHint")).flex_1().min_w_0())
                        // The count grows from the trailing edge, so typing moves nothing else (T8).
                        .child(
                            div()
                                .flex_none()
                                .text_color(if over {
                                    theme.c("red-ink")
                                } else {
                                    theme.c("ink-3")
                                })
                                .when(over, |d| d.font_weight(weight::STRONG))
                                .child(format!(
                                    "{} / {}",
                                    thousands(count, spanish),
                                    thousands(CONTEXT_MAX, spanish)
                                )),
                        ),
                ),
            )
            .child(widgets::status_line(status, &theme).mt_0());

        let foot = widgets::dialog_foot(&theme)
            .child(widgets::button(
                "profile-cancel",
                t(cx, "profile.cancel"),
                Kind::Plain,
                Size::Normal,
                None,
                false,
                &theme,
                |_, w, cx| widgets::close_top(w, cx),
            ))
            .child(widgets::button(
                "profile-save",
                t(cx, "profile.save"),
                Kind::Primary,
                Size::Normal,
                None,
                self.saving || over || heavy || !loaded,
                &theme,
                cx.listener(|this, _, window, cx| this.save(window, cx)),
            ));

        widgets::base(&theme)
            .flex()
            .flex_col()
            .max_h(px(max_h))
            .child(widgets::dialog_head(t(cx, "profile.profile"), &theme))
            .when(!loaded, |d| {
                d.child(
                    div()
                        .mx(px(space::GUTTER))
                        .child(widgets::banner(t(cx, "profile.profileUnavailable"), &theme)),
                )
            })
            .child(
                div()
                    .id("profile-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(space::GUTTER))
                    .child(
                        div()
                            .flex()
                            .gap(px(space::SECTION))
                            .child(self.render_side(loaded, &theme, cx))
                            .child(fields),
                    ),
            )
            .child(foot)
    }
}
