//! The highlight overlay (`cadcraft.highlight`): the host page's boxes painted over the drawing
//! in the shell, after the app has drawn its frame, with the canvas' own world-to-screen
//! transform. Nothing in the app's crates is involved.

use cadcraft_doc::Space;
use cadcraft_geom::Vec2;
use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::canvas::Xf;
use egui::{Color32, Rect, Stroke, StrokeKind, vec2};

use super::protocol::{Bounds, HighlightBox};

/// The colour of a box that names none: an orange that reads on light and dark backgrounds.
pub const DEFAULT_COLOUR: [u8; 3] = [0xff, 0x9f, 0x1a];
/// The smallest a box is drawn (points), so a line (no width) or a point is still seen.
pub const MIN_SIZE: f32 = 12.0;
const OUTLINE: f32 = 2.0;
/// The fill's opacity: faint, so the drawing under it stays readable.
const FILL_ALPHA: u8 = 40;

/// Where a world box appears on screen: the box around its four corners (a twisted view turns
/// it), grown about its centre to at least [`MIN_SIZE`] and cut to just beyond `clip` (so a box
/// larger than the view is not drawn with coordinates past `f32`). `None` when it is entirely
/// outside `clip` or not finite.
pub fn screen_rect(xf: &Xf, bounds: Bounds, clip: Rect) -> Option<Rect> {
    let [x0, y0, x1, y1] = bounds;
    let corners = [Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)].map(|p| xf.to_screen(p));
    if corners.iter().any(|p| p.x.is_nan() || p.y.is_nan()) {
        return None;
    }
    let mut rect = Rect::from_points(&corners);
    if rect.width() < MIN_SIZE || rect.height() < MIN_SIZE {
        let centre = rect.center();
        let size = vec2(rect.width().max(MIN_SIZE), rect.height().max(MIN_SIZE));
        rect = if centre.x.is_finite() && centre.y.is_finite() { Rect::from_center_size(centre, size) } else { rect };
    }
    if !rect.intersects(clip) {
        return None;
    }
    let rect = rect.intersect(clip.expand(OUTLINE * 2.0));
    (rect.is_finite() && rect.is_positive()).then_some(rect)
}

/// Paints `boxes` over the drawing: a 2-point outline and a faint fill in each box's colour,
/// clipped to the drawing area. Only on the Model tab: the boxes are model-space coordinates, and
/// a layout shows paper space.
pub fn paint(painter: &egui::Painter, app: &CadApp, boxes: &[HighlightBox]) {
    if boxes.is_empty() || !app.session.state().is_ok_and(|s| s.space == Space::Model) {
        return;
    }
    let Some(xf) = app.canvas.xf else { return };
    let painter = painter.with_clip_rect(xf.rect);
    for b in boxes {
        let Some(rect) = screen_rect(&xf, b.bounds, xf.rect) else { continue };
        let [r, g, bl] = b.color.unwrap_or(DEFAULT_COLOUR);
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(r, g, bl, FILL_ALPHA));
        painter.rect_stroke(rect, 0.0, Stroke::new(OUTLINE, Color32::from_rgb(r, g, bl)), StrokeKind::Inside);
    }
}

/// The window `zoom: true` fits: the boxes' union with a margin on each side, so they are not
/// against the edge. A union without area (one line, one point) still gets a window: its margin
/// comes from its longer side, else from the drawing's extents, else one drawing unit.
pub fn zoom_window(boxes: &[HighlightBox], extents: Option<Bounds>) -> Option<Bounds> {
    let first = boxes.first()?.bounds;
    let [x0, y0, x1, y1] =
        boxes.iter().fold(first, |[a, b, c, d], h| [a.min(h.bounds[0]), b.min(h.bounds[1]), c.max(h.bounds[2]), d.max(h.bounds[3])]);
    let side = (x1 - x0).max(y1 - y0);
    let drawing = extents.map_or(0.0, |[e0, f0, e1, f1]| (e1 - e0).max(f1 - f0));
    let margin = if side > 0.0 {
        side * 0.1
    } else if drawing > 0.0 {
        drawing * 0.01
    } else {
        1.0
    };
    let window = [x0 - margin, y0 - margin, x1 + margin, y1 + margin];
    window.iter().all(|v| v.is_finite()).then_some(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2 as at, pos2};

    /// A 400 x 200 canvas showing world (0,0) at its centre, 2 points per unit.
    fn xf(twist: f64) -> Xf {
        Xf { rect: Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 200.0)), center: Vec2::new(0.0, 0.0), scale: 2.0, twist }
    }

    fn boxed(bounds: Bounds) -> HighlightBox {
        HighlightBox { bounds, color: None }
    }

    #[test]
    fn a_box_maps_to_screen_with_y_up() {
        let xf = xf(0.0);
        // Centre at (300, 150); x right, y up.
        let r = screen_rect(&xf, [0.0, 0.0, 20.0, 10.0], xf.rect).expect("on screen");
        assert_eq!(r, Rect::from_min_max(at(300.0, 130.0), at(340.0, 150.0)));
    }

    #[test]
    fn a_line_and_a_point_are_drawn_at_the_minimum_size() {
        let xf = xf(0.0);
        let line = screen_rect(&xf, [0.0, 0.0, 20.0, 0.0], xf.rect).expect("on screen");
        assert_eq!((line.width(), line.height()), (40.0, MIN_SIZE));
        assert_eq!(line.center(), at(320.0, 150.0));
        let point = screen_rect(&xf, [5.0, 5.0, 5.0, 5.0], xf.rect).expect("on screen");
        assert_eq!((point.width(), point.height()), (MIN_SIZE, MIN_SIZE));
        assert_eq!(point.center(), at(310.0, 140.0));
    }

    #[test]
    fn a_box_off_the_canvas_is_skipped_and_a_huge_one_is_cut() {
        let xf = xf(0.0);
        assert_eq!(screen_rect(&xf, [1000.0, 1000.0, 1010.0, 1010.0], xf.rect), None);
        let huge = screen_rect(&xf, [-1e300, -1e300, 1e300, 1e300], xf.rect).expect("covers the view");
        assert!(huge.is_finite());
        assert!(huge.contains_rect(xf.rect));
        assert!(xf.rect.expand(OUTLINE * 2.0).contains_rect(huge));
        assert_eq!(screen_rect(&xf, [f64::NAN, 0.0, 1.0, 1.0], xf.rect), None);
    }

    #[test]
    fn a_twisted_view_takes_the_box_around_the_turned_corners() {
        let xf = xf(std::f64::consts::FRAC_PI_2);
        // Turned a quarter: the 20 x 10 box stands 20 points wide and 40 high.
        let r = screen_rect(&xf, [0.0, 0.0, 20.0, 10.0], xf.rect).expect("on screen");
        assert!((r.width() - 20.0).abs() < 1e-3 && (r.height() - 40.0).abs() < 1e-3, "{r:?}");
    }

    #[test]
    fn each_visible_box_is_painted_on_the_model_tab_only() {
        let mut app = CadApp::new(cadcraft_engine::Session::new(), cadcraft_ui_egui::Services::default());
        app.canvas.xf = Some(xf(0.0));
        let shapes = |app: &CadApp, boxes: &[HighlightBox]| {
            let ctx = egui::Context::default();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| paint(ui.painter(), app, boxes));
            // The font atlas is uploaded with the first frame; a test has no GPU to take it.
            out.textures_delta.clear();
            out.shapes.len()
        };
        let none = shapes(&app, &[]);
        let red = HighlightBox { bounds: [0.0, 0.0, 1.0, 1.0], color: Some([255, 0, 0]) };
        // A fill and an outline for the box on screen; nothing for the one off it.
        assert_eq!(shapes(&app, &[red, boxed([1000.0, 1000.0, 1001.0, 1001.0])]), none + 2);
        app.session.execute("layout.set", &serde_json::json!({"name": "Layout1"})).expect("a layout");
        assert_eq!(shapes(&app, &[red]), none);
    }

    /// In a real frame, the boxes come after everything the canvas painted (shapes are returned
    /// in paint order): they are on top of the drawing, not under it.
    #[test]
    fn the_boxes_are_painted_over_the_drawing() {
        let mut app = CadApp::new(cadcraft_engine::Session::new(), cadcraft_ui_egui::Services::default());
        app.session.execute("line", &serde_json::json!({"points": [[0, 0], [10, 10]]})).expect("line");
        app.canvas.zoom_pending = true;
        let red = HighlightBox { bounds: [0.0, 0.0, 10.0, 10.0], color: Some([255, 0, 0]) };
        let ctx = egui::Context::default();
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
                paint(ui.painter(), &app, &[red]);
            });
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let canvas = app.canvas.rect.expect("the canvas was laid out");
        let is_ours = |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == Color32::from_rgb(255, 0, 0) || r.fill == Color32::from_rgba_unmultiplied(255, 0, 0, FILL_ALPHA));
        let ours: Vec<usize> = shapes.iter().enumerate().filter(|(_, s)| is_ours(s)).map(|(i, _)| i).collect();
        assert_eq!(ours.len(), 2, "a fill and an outline");
        let last_canvas = shapes.iter().rposition(|s| s.clip_rect == canvas && !is_ours(s)).expect("the canvas painted the drawing");
        assert!(ours[0] > last_canvas, "overlay at {ours:?}, the canvas painted up to {last_canvas}");
    }

    #[test]
    fn zoom_fits_the_union_with_a_margin() {
        assert_eq!(zoom_window(&[], None), None);
        assert_eq!(zoom_window(&[boxed([0.0, 0.0, 10.0, 10.0]), boxed([20.0, 0.0, 30.0, 5.0])], None), Some([-3.0, -3.0, 33.0, 13.0]));
        // A horizontal line: the margin also gives it height.
        assert_eq!(zoom_window(&[boxed([0.0, 0.0, 10.0, 0.0])], None), Some([-1.0, -1.0, 11.0, 1.0]));
        // A point: a hundredth of the drawing, or one unit without extents.
        assert_eq!(zoom_window(&[boxed([5.0, 5.0, 5.0, 5.0])], Some([0.0, 0.0, 1000.0, 400.0])), Some([-5.0, -5.0, 15.0, 15.0]));
        assert_eq!(zoom_window(&[boxed([5.0, 5.0, 5.0, 5.0])], None), Some([4.0, 4.0, 6.0, 6.0]));
    }
}
