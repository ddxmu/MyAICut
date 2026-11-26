//! Visual feedback for real AI editor commands. Every pointer represents an actual command target;
//! the overlay never sends mouse events or changes the editor itself.
use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::Value;

use crate::{FilmcraftApp, automation::Registry, theme::Tokens};

pub(super) const ARRIVAL: Duration = Duration::from_millis(480);
const LINGER: Duration = Duration::from_millis(1850);
const MAX_POINTERS: usize = 12;
const COLORS: [Color32; 6] = [
    Color32::from_rgb(93, 213, 255),
    Color32::from_rgb(193, 137, 255),
    Color32::from_rgb(102, 236, 173),
    Color32::from_rgb(255, 193, 98),
    Color32::from_rgb(255, 129, 190),
    Color32::from_rgb(119, 166, 255),
];

#[derive(Default)]
pub(super) struct Activity {
    pointers: Vec<Pointer>,
    serial: usize,
}

struct Pointer {
    target: Rect,
    point: Pos2,
    origin: Pos2,
    label: String,
    color: Color32,
    start: Instant,
    result: Option<(Instant, bool)>,
}

fn rect(auto: &Registry, id: &str) -> Option<Rect> {
    let element = auto.find(id)?;
    if element.window.is_some() || element.rect.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let [x, y, w, h] = element.rect;
    (w > 1.0 && h > 1.0).then(|| Rect::from_min_size(pos2(x, y), vec2(w, h)))
}

fn is_clip_command(id: &str) -> bool {
    ["timeline.", "clip.", "edit.", "effects.", "effect.", "graphics.", "lumetri.", "audio.", "transition.", "caption."].iter().any(|p| id.starts_with(p))
}

fn clip_ids(params: &Value) -> Vec<u64> {
    let mut ids = Vec::new();
    let mut add = |value: &Value| {
        if let Some(id) = value.as_u64()
            && !ids.contains(&id)
            && ids.len() < MAX_POINTERS
        {
            ids.push(id);
        }
    };
    add(&params["clip"]);
    if let Some(clips) = params["clips"].as_array() {
        for clip in clips {
            add(clip);
        }
    }
    if let Some(moves) = params["moves"].as_array() {
        for movement in moves {
            add(&movement["clip"]);
        }
    }
    ids
}

fn targets(app: &FilmcraftApp, id: &str, params: &Value) -> Vec<(Rect, Pos2)> {
    let mut ids = clip_ids(params);
    let mut cut_x = None;
    let cut = matches!(id, "sequence.addEdit" | "sequence.addEditAllTracks" | "timeline.razor");
    if cut {
        let seconds = params["time"]
            .as_i64()
            .map(|time| time as f64 / 254_016_000_000.0)
            .or_else(|| params["frame"].as_i64().map(|f| app.session.sequence_rate().tick_of(f).seconds()))
            .or_else(|| params["seconds"].as_f64())
            .unwrap_or_else(|| app.session.playhead().seconds());
        if let Some(ruler) = rect(&app.auto, "timeline.ruler") {
            cut_x = Some(ruler.left() + ((seconds - app.ui.timeline.scroll) * app.ui.timeline.pps) as f32);
        }
        if let Some(sequence) = app.session.active_sequence() {
            let selected = &app.session.state.selection;
            let targeted = app.session.targeting().targeted;
            let selected_here = id == "sequence.addEdit"
                && sequence.video_tracks.iter().chain(&sequence.audio_tracks).any(|track| {
                    track.items.iter().any(|item| selected.contains(&item.id) && item.start.seconds() < seconds && seconds < item.end().seconds())
                });
            let explicit = ids.clone();
            ids.clear();
            for track in sequence.video_tracks.iter().chain(&sequence.audio_tracks) {
                if track.locked {
                    continue;
                }
                if id == "sequence.addEdit" && !selected_here && !targeted.contains(&track.id) {
                    continue;
                }
                if id == "timeline.razor"
                    && params.get("track").is_some()
                    && params["track"].as_u64() != Some(track.id.0)
                    && params["track"].as_str() != Some(track.name.as_str())
                {
                    continue;
                }
                for item in &track.items {
                    if item.start.seconds() < seconds
                        && seconds < item.end().seconds()
                        && (!selected_here || selected.contains(&item.id))
                        && (explicit.is_empty() || explicit.contains(&item.id.0))
                        && ids.len() < MAX_POINTERS
                    {
                        ids.push(item.id.0);
                    }
                }
            }
        }
    } else if ids.is_empty() && is_clip_command(id) && id != "timeline.place" {
        ids.extend(app.session.state.selection.iter().take(MAX_POINTERS).map(|clip| clip.0));
    }
    let mut result: Vec<(Rect, Pos2)> = ids
        .into_iter()
        .filter_map(|clip| {
            let target = rect(&app.auto, &format!("timeline.clip.{clip}"))?;
            let mut point = target.center();
            if let Some(x) = cut_x.filter(|x| target.left() <= *x && *x <= target.right()) {
                point.x = x;
            }
            if id == "timeline.trim" {
                point.x = if params["edge"] == "in" { target.left() } else { target.right() };
            }
            Some((target, point))
        })
        .collect();
    if id == "timeline.place" {
        let items = params["items"]
            .as_array()
            .map(|list| list.iter().filter_map(Value::as_u64).take(MAX_POINTERS).collect::<Vec<_>>())
            .unwrap_or_else(|| params["item"].as_u64().into_iter().collect());
        for item in items {
            if let Some(el) = app.auto.all().iter().find(|e| e.id.ends_with(&format!(".item.{item}")) && e.window.is_none())
                && let Some(r) = rect(&app.auto, &el.id)
            {
                result.push((r, r.center()));
            }
        }
    }
    if result.is_empty() {
        let panel = if id.starts_with("file.export") || id.starts_with("export.") {
            "header.mode.export"
        } else if id.starts_with("project.") || id.starts_with("file.import") {
            "panel.Project"
        } else if id.starts_with("effects.") || id.starts_with("effect.") {
            "panel.EffectControls"
        } else if is_clip_command(id) || id.starts_with("sequence.") || id.starts_with("source.") {
            "timeline.ruler"
        } else {
            "header.ai"
        };
        if let Some(r) = rect(&app.auto, panel) {
            let x = cut_x.filter(|x| r.left() <= *x && *x <= r.right()).unwrap_or(r.center().x);
            result.push((r, pos2(x, r.center().y)));
        }
    }
    result.truncate(MAX_POINTERS);
    result
}

impl Activity {
    pub(super) fn begin(&mut self, app: &FilmcraftApp, id: &str, params: &Value, now: Instant) -> bool {
        if filmcraft_engine::find_command(id).is_some_and(|command| !command.journal) {
            return false;
        }
        self.pointers.retain(|p| p.result.is_some_and(|(end, _)| now.saturating_duration_since(end) < LINGER));
        let label = filmcraft_engine::find_command(id)
            .map(|c| c.label.to_owned())
            .or_else(|| crate::menus::external_commands().iter().find(|c| c.id == id).map(|c| c.label.clone()))
            .unwrap_or_else(|| id.to_owned());
        let mut label = crate::i18n::t(&label).to_owned();
        label.truncate(label.floor_char_boundary(100));
        let points = targets(app, id, params);
        let count = points.len();
        for (target, point) in points {
            let color = COLORS[self.serial % COLORS.len()];
            self.serial = self.serial.wrapping_add(1);
            let origin = point + vec2(if self.serial % 2 == 0 { -46.0 } else { 46.0 }, -52.0);
            self.pointers.push(Pointer { target, point, origin, label: label.clone(), color, start: now, result: None });
        }
        if self.pointers.len() > MAX_POINTERS {
            let excess = self.pointers.len() - MAX_POINTERS;
            self.pointers.drain(..excess);
        }
        count > 0
    }

    pub(super) fn finish(&mut self, ok: bool, now: Instant) {
        for pointer in self.pointers.iter_mut().filter(|p| p.result.is_none()) {
            pointer.result = Some((now, ok));
        }
    }

    pub(super) fn clear(&mut self) {
        self.pointers.clear();
    }

    pub(super) fn paint(&mut self, ctx: &egui::Context, auto: &mut Registry) {
        let now = Instant::now();
        self.pointers.retain(|p| p.result.is_none_or(|(end, _)| now.saturating_duration_since(end) < LINGER));
        if self.pointers.is_empty() {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("myaicut-ai-activity"))).with_clip_rect(screen);
        for (index, p) in self.pointers.iter().enumerate() {
            let phase = (now.saturating_duration_since(p.start).as_secs_f32() / ARRIVAL.as_secs_f32()).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - phase).powi(3);
            let finished = p.result.map(|(end, ok)| (now.saturating_duration_since(end).as_secs_f32(), ok));
            let opacity = finished.map_or(1.0, |(age, _)| ((LINGER.as_secs_f32() - age) / 0.65).clamp(0.0, 1.0));
            let tint = if finished.is_some_and(|(_, ok)| !ok) { Color32::from_rgb(255, 100, 112) } else { p.color };
            let color = tint.gamma_multiply(opacity);
            let tip = p.origin.lerp(p.point, ease) + vec2(0.0, -(phase * std::f32::consts::PI).sin() * 15.0);
            let highlight = p.target.intersect(screen);
            if highlight.is_positive() {
                painter.rect_filled(highlight, 3.0, tint.gamma_multiply(0.10 * opacity));
                painter.rect_stroke(highlight, 3.0, Stroke::new(1.5, color), StrokeKind::Inside);
            }
            if !screen.contains(tip) {
                continue;
            }
            painter.line_segment([p.origin.lerp(p.point, (ease - 0.2).max(0.0)), tip], Stroke::new(1.5, color.gamma_multiply(0.55)));
            let ring = finished.map_or(7.0 + 5.0 * phase, |(age, _)| 8.0 + 20.0 * age.min(0.5));
            painter.circle_stroke(tip, ring, Stroke::new(1.3, color.gamma_multiply(0.55)));
            let shaft = [tip + vec2(5.5, 10.0), tip + vec2(13.0, 22.0)];
            painter.line_segment(shaft, Stroke::new(6.0, Color32::BLACK.gamma_multiply(opacity)));
            painter.line_segment(shaft, Stroke::new(4.0, color));
            painter.add(egui::Shape::convex_polygon(
                vec![tip, tip + vec2(0.0, 19.0), tip + vec2(18.0, 11.0)],
                color,
                Stroke::new(1.0, Color32::BLACK.gamma_multiply(opacity)),
            ));
            let state = match finished {
                Some((_, true)) => "✓",
                Some((_, false)) => "×",
                None => "•",
            };
            let galley = painter.layout_no_wrap(format!("AI {state} {}", p.label), Tokens::ui(11.0), Color32::WHITE.gamma_multiply(opacity));
            let size = galley.size() + vec2(12.0, 6.0);
            let tag = Rect::from_min_size(
                pos2((tip.x + 14.0).min((screen.right() - size.x).max(screen.left())), (tip.y + 23.0).min((screen.bottom() - size.y).max(screen.top()))),
                size,
            );
            painter.rect_filled(tag, 5.0, Color32::from_rgb(24, 27, 35).gamma_multiply(opacity));
            painter.rect_stroke(tag, 5.0, Stroke::new(1.0, color), StrokeKind::Inside);
            painter.galley(tag.min + vec2(6.0, 3.0), galley, Color32::WHITE.gamma_multiply(opacity));
            auto.add(&format!("ai.activity.cursor.{index}"), tag, &p.label);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
