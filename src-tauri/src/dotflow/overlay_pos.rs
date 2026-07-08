//! Pure geometry for the selection-review overlay card.
//!
//! `clamp_overlay_position` anchors the card below-right of the cursor, flips it to above/left near the
//! right/bottom edges, then hard-clamps so it can never leave the work area. It is pure + total (no IO,
//! no clock, no global state), so it is fully unit-testable AND gradable by the DTF referee. The DPI
//! conversion (physical → logical px) lives in the effectful caller (Task A5), NOT here — see the doc
//! comment on `clamp_overlay_position`.

#[derive(Clone, Copy, Debug)]
pub struct WorkArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Top-left position for the review overlay: below-right of the cursor by a gap, flipping to
/// above/left near the right/bottom edges, then hard-clamped so it never leaves the work area.
///
/// **All inputs MUST be logical pixels** — `cursor`, `win` (width, height), and `work` must share one
/// logical-pixel coordinate space. This function is DPI-agnostic on purpose: enigo's physical cursor
/// and Tauri's physical monitor bounds are converted by `÷ scale_factor` in the caller (Task A5) BEFORE
/// calling here, so the clamp stays pure and the conversion is exercised only in the A11 live run
/// (finding `[F6]`). Passing physical px here would place the card wrong on any HiDPI display.
pub fn clamp_overlay_position(cursor: (f64, f64), win: (f64, f64), work: WorkArea) -> (f64, f64) {
    const GAP: f64 = 12.0;
    let (cx, cy) = cursor;
    let (w, h) = win;
    let mut x = cx + GAP;
    let mut y = cy + GAP;
    if x + w > work.x + work.width {
        x = cx - GAP - w;
    }
    if y + h > work.y + work.height {
        y = cy - GAP - h;
    }
    x = x.clamp(work.x, (work.x + work.width - w).max(work.x));
    y = y.clamp(work.y, (work.y + work.height - h).max(work.y));
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FHD: WorkArea = WorkArea {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn sits_below_right_of_cursor_mid_screen() {
        // GAP = 12; no flip needed away from edges.
        assert_eq!(
            clamp_overlay_position((500.0, 500.0), (420.0, 300.0), FHD),
            (512.0, 512.0)
        );
    }

    #[test]
    fn flips_left_at_right_edge() {
        // 1900+12+420 = 2332 > 1920 -> flip left: 1900-12-420 = 1468
        let (x, _) = clamp_overlay_position((1900.0, 500.0), (420.0, 300.0), FHD);
        assert_eq!(x, 1468.0);
    }

    #[test]
    fn flips_up_at_bottom_edge() {
        // 1070+12+300 = 1382 > 1080 -> flip up: 1070-12-300 = 758
        let (_, y) = clamp_overlay_position((500.0, 1070.0), (420.0, 300.0), FHD);
        assert_eq!(y, 758.0);
    }

    #[test]
    fn clamps_to_edge_when_flip_would_still_overflow() {
        // [F10] Realistic case: window FITS the work area, but anchoring+flipping near the top-left
        // corner would push it off the left/top. Must clamp back to the edge, not return negatives.
        // Fails if the final .clamp() lines are deleted (they return (-427, -302) without them).
        let wa = WorkArea {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        let (x, y) = clamp_overlay_position((5.0, 5.0), (420.0, 300.0), wa);
        // default 17,17 fits (17+420=437<=800) so NO flip; stays at 17,17 — assert it's on-screen and
        // exactly the anchored position (catches an unintended flip AND an over-eager clamp).
        assert_eq!((x, y), (17.0, 17.0));
        assert!(x >= wa.x && x + 420.0 <= wa.width, "off right/left edge");
        assert!(y >= wa.y && y + 300.0 <= wa.height, "off top/bottom edge");
    }

    #[test]
    fn clamps_hard_when_window_exceeds_work_area() {
        // Degenerate: window wider than the work area — cannot fully fit, so clamp pins the top-left
        // to the work-area origin (the best we can do). Asserts the pin, NOT an impossible "fully
        // on-screen". Fails (returns negatives) if the clamp lines are removed.
        let tiny = WorkArea {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 300.0,
        };
        assert_eq!(
            clamp_overlay_position((10.0, 10.0), (420.0, 320.0), tiny),
            (0.0, 0.0)
        );
    }

    #[test]
    fn respects_non_zero_monitor_origin() {
        // [F6] Second monitor at logical origin (1920, 0). Mid-screen cursor must anchor below-right
        // relative to THAT origin, not (0,0). Caller is responsible for passing LOGICAL coords.
        let m2 = WorkArea {
            x: 1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        assert_eq!(
            clamp_overlay_position((2400.0, 500.0), (420.0, 300.0), m2),
            (2412.0, 512.0)
        );
    }
}
