//! Unicode rendering backend for the full-screen graph view (F10).
//!
//! Produces plain ratatui `Buffer` output using block characters for
//! bar charts and single-character sparkline dots for line charts.
//! Other graph types show a placeholder until the raster path (slice 5)
//! lands.
//!
//! Values arrive pre-resolved as [`GraphValues`]; this crate intentionally
//! knows nothing about [`Engine`](l123_engine) — the UI layer walks the
//! ranges and hands numbers in.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};

use crate::{GraphDef, GraphType};

/// Numeric values resolved per series slot.
///
/// `None` means the corresponding series has no range set. An empty
/// `Vec` means the range is set but yielded no numeric cells. Cells
/// that are blank or non-numeric show up as `f64::NAN` so the positional
/// alignment between series is preserved.
#[derive(Clone, Debug, Default)]
pub struct GraphValues {
    pub x: Option<Vec<f64>>,
    pub data: [Option<Vec<f64>>; 6],
}

impl GraphValues {
    /// True when every series slot (including X) is unset. A graph
    /// with no data is not worth rendering; the caller should show
    /// a "Define ranges first" message instead.
    pub fn is_empty(&self) -> bool {
        self.x.is_none() && self.data.iter().all(Option::is_none)
    }

    /// The first data series that has any numeric values, in A..F order.
    pub fn first_series(&self) -> Option<&[f64]> {
        self.data.iter().find_map(|opt| opt.as_deref())
    }
}

/// Render a graph into `buf` over `area`. Falls back to a one-line
/// placeholder for types not yet implemented (slice 5 fills in).
pub fn render(def: &GraphDef, vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    clear(area, buf);
    if area.width < 8 || area.height < 4 {
        return;
    }
    if vals.is_empty() {
        write_centered(
            area,
            buf,
            "No graph ranges set — /Graph X and /Graph A..F define them.",
        );
        return;
    }
    let with_titles = reserve_title_rows(area, &def.options.titles, buf);
    let plot_area = reserve_legend_row(with_titles, &def.options.legend, buf);
    if plot_area.height < 3 {
        // No room left for a meaningful plot.
        return;
    }
    // Frame goes around the plot rectangle; the inner area is what
    // grid + data get to draw into so bars / dots don't overdraw the
    // edges. Default is all four sides on (Reference p. 2-198).
    let inner = render_frame(&def.features.frame, plot_area, buf);
    if inner.height < 2 {
        return;
    }
    // Grid is painted first so the per-type renderers overwrite it
    // where their bars/dots fall. The dotted unicode glyphs sit
    // beneath the data without competing for visual weight.
    render_grid_lines(&def.options.grid, inner, buf);
    match def.graph_type {
        GraphType::Bar => match def.features.orientation {
            crate::Orientation::Vertical => render_bar(vals, inner, buf),
            crate::Orientation::Horizontal => render_bar_horizontal(vals, inner, buf),
        },
        GraphType::Line => render_line(vals, inner, buf),
        other => write_centered(
            inner,
            buf,
            &format!("{other:?} graphs render in a later slice; press Esc to return."),
        ),
    }
}

/// Paint the frame edges per `features.frame` and return the inner
/// rectangle the data is allowed to fill. Each enabled edge consumes
/// one row or column. Corner glyphs (`┌┐└┘`) only appear where both
/// adjacent edges are on; otherwise the corner cell continues
/// whichever edge is on (or stays blank).
fn render_frame(frame: &crate::FrameMask, area: Rect, buf: &mut Buffer) -> Rect {
    let style = Style::default().fg(Color::White);
    let left_x = area.left();
    let right_x = area.right().saturating_sub(1);
    let top_y = area.top();
    let bottom_y = area.bottom().saturating_sub(1);

    // Edges (excluding corners, which are written below).
    if frame.top && area.height >= 1 {
        for x in (left_x + 1)..right_x {
            buf[(x, top_y)].set_symbol("─");
            buf[(x, top_y)].set_style(style);
        }
    }
    if frame.bottom && area.height >= 2 {
        for x in (left_x + 1)..right_x {
            buf[(x, bottom_y)].set_symbol("─");
            buf[(x, bottom_y)].set_style(style);
        }
    }
    if frame.left && area.width >= 1 {
        for y in (top_y + 1)..bottom_y {
            buf[(left_x, y)].set_symbol("│");
            buf[(left_x, y)].set_style(style);
        }
    }
    if frame.right && area.width >= 2 {
        for y in (top_y + 1)..bottom_y {
            buf[(right_x, y)].set_symbol("│");
            buf[(right_x, y)].set_style(style);
        }
    }
    // Corners.
    if area.width >= 1 && area.height >= 1 {
        let glyph = corner_glyph(frame.top, frame.left, "┌", "─", "│");
        if !glyph.is_empty() {
            buf[(left_x, top_y)].set_symbol(glyph);
            buf[(left_x, top_y)].set_style(style);
        }
    }
    if area.width >= 2 && area.height >= 1 {
        let glyph = corner_glyph(frame.top, frame.right, "┐", "─", "│");
        if !glyph.is_empty() {
            buf[(right_x, top_y)].set_symbol(glyph);
            buf[(right_x, top_y)].set_style(style);
        }
    }
    if area.width >= 1 && area.height >= 2 {
        let glyph = corner_glyph(frame.bottom, frame.left, "└", "─", "│");
        if !glyph.is_empty() {
            buf[(left_x, bottom_y)].set_symbol(glyph);
            buf[(left_x, bottom_y)].set_style(style);
        }
    }
    if area.width >= 2 && area.height >= 2 {
        let glyph = corner_glyph(frame.bottom, frame.right, "┘", "─", "│");
        if !glyph.is_empty() {
            buf[(right_x, bottom_y)].set_symbol(glyph);
            buf[(right_x, bottom_y)].set_style(style);
        }
    }

    // Compute inner rectangle by inset on every enabled side.
    let inset_top = if frame.top { 1 } else { 0 };
    let inset_bottom = if frame.bottom { 1 } else { 0 };
    let inset_left = if frame.left { 1 } else { 0 };
    let inset_right = if frame.right { 1 } else { 0 };
    Rect::new(
        area.x + inset_left,
        area.y + inset_top,
        area.width.saturating_sub(inset_left + inset_right),
        area.height.saturating_sub(inset_top + inset_bottom),
    )
}

/// Pick the glyph for one corner: both edges on → corner; only the
/// horizontal edge on → `─`; only the vertical → `│`; neither →
/// empty (caller leaves the cell blank).
fn corner_glyph(
    horizontal: bool,
    vertical: bool,
    corner: &'static str,
    h_only: &'static str,
    v_only: &'static str,
) -> &'static str {
    match (horizontal, vertical) {
        (true, true) => corner,
        (true, false) => h_only,
        (false, true) => v_only,
        (false, false) => "",
    }
}

/// Draw light dotted grid lines into `area`, picking three evenly-
/// spaced positions in each direction. The plot baseline (last row)
/// is left untouched so the per-type renderers can still draw their
/// own axis line over it. Glyph choices: `┄` (U+2504) for
/// horizontal, `┊` (U+250A) for vertical — distinct from `─` and `│`
/// so substring tests can tell grid from axis.
fn render_grid_lines(grid: &crate::GridMask, area: Rect, buf: &mut Buffer) {
    if !grid.horizontal && !grid.vertical {
        return;
    }
    let style = Style::default().fg(Color::DarkGray);
    let inner_height = area.height.saturating_sub(1); // skip baseline row
    if grid.horizontal && inner_height >= 4 {
        for n in 1..=3 {
            let y = area.top() + (inner_height as u32 * n as u32 / 4) as u16;
            if y >= area.bottom().saturating_sub(1) {
                continue;
            }
            for x in area.left()..area.right() {
                let cell = &mut buf[(x, y)];
                cell.set_symbol("┄");
                cell.set_style(style);
            }
        }
    }
    if grid.vertical && area.width >= 4 {
        for n in 1..=3 {
            let x = area.left() + (area.width as u32 * n as u32 / 4) as u16;
            if x >= area.right() {
                continue;
            }
            for y in area.top()..area.bottom().saturating_sub(1) {
                let cell = &mut buf[(x, y)];
                // Only paint where horizontal didn't already write a
                // glyph, to avoid the visual cross at intersections.
                let existing = cell.symbol();
                if existing == " " {
                    cell.set_symbol("┊");
                    cell.set_style(style);
                }
            }
        }
    }
}

/// Paint the legend row and shrink `area` upward by one row when at
/// least one legend slot is set. The row is laid out left-aligned as
/// `A: name  B: name  …`, skipping unset slots. Truncates to fit the
/// area width.
fn reserve_legend_row(area: Rect, legend: &[Option<String>; 6], buf: &mut Buffer) -> Rect {
    let parts: Vec<String> = legend
        .iter()
        .enumerate()
        .filter_map(|(i, opt)| {
            opt.as_deref()
                .map(|name| format!("{}: {}", (b'A' + i as u8) as char, name))
        })
        .collect();
    if parts.is_empty() || area.height < 4 {
        return area;
    }
    let mut joined = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            joined.push_str("  ");
        }
        joined.push_str(p);
    }
    let max_len = area.width.saturating_sub(2) as usize;
    if joined.chars().count() > max_len {
        joined = joined.chars().take(max_len).collect();
    }
    let y = area.bottom().saturating_sub(1);
    let style = Style::default().fg(Color::White);
    buf.set_string(area.left() + 1, y, joined, style);
    Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1))
}

/// Paint First / Second / X-Axis titles into `area` and return the
/// remaining rectangle the plot is allowed to occupy. Y-Axis,
/// 2Y-Axis, Note, and Other-Note are deferred to a later slice.
fn reserve_title_rows(area: Rect, titles: &crate::Titles, buf: &mut Buffer) -> Rect {
    let mut top = area.top();
    let mut bottom = area.bottom();
    let style = Style::default().fg(Color::White);
    if let Some(t) = titles.first.as_deref() {
        if top < bottom {
            write_centered_row(area, top, buf, t, style);
            top = top.saturating_add(1);
        }
    }
    if let Some(t) = titles.second.as_deref() {
        if top < bottom {
            write_centered_row(area, top, buf, t, style);
            top = top.saturating_add(1);
        }
    }
    if let Some(t) = titles.x_axis.as_deref() {
        if bottom > top {
            bottom = bottom.saturating_sub(1);
            write_centered_row(area, bottom, buf, t, style);
        }
    }
    Rect::new(area.x, top, area.width, bottom.saturating_sub(top))
}

fn write_centered_row(area: Rect, y: u16, buf: &mut Buffer, msg: &str, style: Style) {
    let len = msg.chars().count() as u16;
    let max_len = area.width.saturating_sub(2);
    let display: String = if len > max_len {
        msg.chars().take(max_len as usize).collect()
    } else {
        msg.to_owned()
    };
    let display_len = display.chars().count() as u16;
    let x0 = area.left() + area.width.saturating_sub(display_len) / 2;
    buf.set_string(x0, y, display, style);
}

fn clear(area: Rect, buf: &mut Buffer) {
    let blank = Style::default().bg(Color::Reset).fg(Color::Reset);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.set_style(blank);
        }
    }
}

fn write_centered(area: Rect, buf: &mut Buffer, msg: &str) {
    let y = area.top() + area.height / 2;
    let x0 = area.left() + area.width.saturating_sub(msg.chars().count() as u16) / 2;
    buf.set_string(x0, y, msg, Style::default());
}

/// Half-block bar chart. One column per A-series value. Height in
/// half-rows is `2 * (row_height) * val / max_abs`, rounded; odd half
/// steps use `▄` (lower half block) / `▀` (upper half block) for the
/// fractional cell at the top of the bar.
fn render_bar(vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some(series) = vals.first_series() else {
        write_centered(area, buf, "No numeric A-series values to plot.");
        return;
    };
    // Leave the bottom row for the baseline axis.
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 || series.is_empty() {
        return;
    }
    let max = series
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    let max = if max <= 0.0 || !max.is_finite() {
        1.0
    } else {
        max
    };

    let bar_count = series.len() as u16;
    let plot_width = area.width;
    // One column per bar, one column of padding between them when we
    // can afford it.
    let step = (plot_width / bar_count).max(1);
    let bar_width = step.saturating_sub(1).max(1);

    let full = "█";
    let half = "▄";
    let style = Style::default().fg(Color::Cyan);

    for (i, v) in series.iter().copied().enumerate() {
        if !v.is_finite() || v <= 0.0 {
            continue;
        }
        let x0 = area.left() + (i as u16) * step;
        let height_halves = ((v / max) * plot_height as f64 * 2.0).round() as u16;
        let full_rows = height_halves / 2;
        let has_half = height_halves % 2 == 1;
        for row in 0..full_rows {
            let y = plot_bottom.saturating_sub(1 + row);
            for bx in 0..bar_width {
                let x = x0 + bx;
                if x < area.right() && y >= area.top() {
                    let cell = &mut buf[(x, y)];
                    cell.set_symbol(full);
                    cell.set_style(style);
                }
            }
        }
        if has_half {
            let y = plot_bottom.saturating_sub(1 + full_rows);
            for bx in 0..bar_width {
                let x = x0 + bx;
                if x < area.right() && y >= area.top() {
                    let cell = &mut buf[(x, y)];
                    cell.set_symbol(half);
                    cell.set_style(style);
                }
            }
        }
    }
    // Baseline row of `─`.
    for x in area.left()..area.right() {
        let cell = &mut buf[(x, plot_bottom)];
        cell.set_symbol("─");
        cell.set_style(Style::default().fg(Color::Gray));
    }
}

/// Half-block bar chart laid on its side. One ROW per A-series
/// value; bar length grows from the left baseline rightward in
/// proportion to the value. Half-cells at the right end use `▌`
/// (LEFT HALF BLOCK), which fills the left half of the cell so the
/// bar appears to stop mid-cell.
fn render_bar_horizontal(vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some(series) = vals.first_series() else {
        write_centered(area, buf, "No numeric A-series values to plot.");
        return;
    };
    // Leave the left column for the baseline axis.
    let plot_left = area.left().saturating_add(1);
    let plot_right = area.right();
    let plot_width = plot_right.saturating_sub(plot_left);
    if plot_width < 2 || area.height < 2 || series.is_empty() {
        return;
    }
    let max = series
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    let max = if max <= 0.0 || !max.is_finite() {
        1.0
    } else {
        max
    };

    let bar_count = series.len() as u16;
    let plot_height = area.height;
    let step = (plot_height / bar_count).max(1);
    let bar_height = step.saturating_sub(1).max(1);

    let full = "█";
    let half = "▌";
    let style = Style::default().fg(Color::Cyan);

    for (i, v) in series.iter().copied().enumerate() {
        if !v.is_finite() || v <= 0.0 {
            continue;
        }
        let y0 = area.top() + (i as u16) * step;
        let length_halves = ((v / max) * plot_width as f64 * 2.0).round() as u16;
        let full_cols = length_halves / 2;
        let has_half = length_halves % 2 == 1;
        for ry in 0..bar_height {
            let y = y0 + ry;
            if y >= area.bottom() {
                break;
            }
            for col in 0..full_cols {
                let x = plot_left + col;
                if x >= plot_right {
                    break;
                }
                let cell = &mut buf[(x, y)];
                cell.set_symbol(full);
                cell.set_style(style);
            }
            if has_half {
                let x = plot_left + full_cols;
                if x < plot_right {
                    let cell = &mut buf[(x, y)];
                    cell.set_symbol(half);
                    cell.set_style(style);
                }
            }
        }
    }
    // Vertical baseline column of `│` on the left.
    let baseline_x = area.left();
    if baseline_x < area.right() {
        for y in area.top()..area.bottom() {
            let cell = &mut buf[(baseline_x, y)];
            cell.set_symbol("│");
            cell.set_style(Style::default().fg(Color::Gray));
        }
    }
}

/// Dot-per-sample line chart. Each A-series point is a `•` placed
/// at its y-position. Spans the full plot width evenly regardless
/// of how many samples there are.
fn render_line(vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some(series) = vals.first_series() else {
        write_centered(area, buf, "No numeric A-series values to plot.");
        return;
    };
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 || series.is_empty() {
        return;
    }
    let (min, max) = series
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    let (min, max) = if !min.is_finite() || !max.is_finite() || min == max {
        (0.0, 1.0)
    } else {
        (min, max)
    };
    let style = Style::default().fg(Color::Cyan);
    let span = max - min;
    let n = series.len().max(1);
    let denom = (n - 1).max(1) as f64;
    for (i, v) in series.iter().copied().enumerate() {
        if !v.is_finite() {
            continue;
        }
        let frac_x = if n == 1 { 0.5 } else { i as f64 / denom };
        let x = area.left() + (frac_x * (area.width as f64 - 1.0)).round() as u16;
        let frac_y = (v - min) / span;
        let y_from_bottom = (frac_y * (plot_height as f64 - 1.0)).round() as u16;
        let y = plot_bottom.saturating_sub(1 + y_from_bottom);
        if x < area.right() && y >= area.top() && y < area.bottom() {
            let cell = &mut buf[(x, y)];
            cell.set_symbol("•");
            cell.set_style(style);
        }
    }
    for x in area.left()..area.right() {
        let cell = &mut buf[(x, plot_bottom)];
        cell.set_symbol("─");
        cell.set_style(Style::default().fg(Color::Gray));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphFeatures, Orientation};

    fn render_to(def: GraphType, a: Vec<f64>, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let mut vals = GraphValues::default();
        vals.data[0] = Some(a);
        let gdef = GraphDef {
            graph_type: def,
            ..Default::default()
        };
        render(&gdef, &vals, area, &mut buf);
        buf
    }

    fn contains(buf: &Buffer, needle: &str) -> bool {
        for y in 0..buf.area.height {
            let mut row = String::new();
            for x in 0..buf.area.width {
                row.push_str(buf[(x, y)].symbol());
            }
            if row.contains(needle) {
                return true;
            }
        }
        false
    }

    #[test]
    fn bar_renders_full_block() {
        let buf = render_to(GraphType::Bar, vec![1.0, 2.0, 3.0], 40, 10);
        assert!(contains(&buf, "█"), "no █ in bar chart");
        assert!(contains(&buf, "─"), "no baseline");
    }

    #[test]
    fn bar_horizontal_at_acceptance_size() {
        // Mirror the acceptance harness: 80x30 buffer, body region
        // is rows 3..29 (chunks[1] in App::render at SIZE 80 30).
        let buf_area = Rect::new(0, 0, 80, 30);
        let body_area = Rect::new(0, 3, 80, 26);
        let mut buf = Buffer::empty(buf_area);
        let mut vals = GraphValues::default();
        vals.data[0] = Some(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let def = GraphDef {
            graph_type: GraphType::Bar,
            features: GraphFeatures {
                orientation: Orientation::Horizontal,
                ..Default::default()
            },
            ..Default::default()
        };
        render(&def, &vals, body_area, &mut buf);
        let mut got = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                got.push_str(buf[(x, y)].symbol());
            }
            got.push('\n');
        }
        assert!(
            got.contains("▌"),
            "no ▌ at acceptance size; rendered:\n{got}"
        );
    }

    #[test]
    fn bar_horizontal_renders_left_to_right() {
        // Pick widths that guarantee a half-block at least once.
        let area = Rect::new(0, 0, 41, 10);
        let mut buf = Buffer::empty(area);
        let mut vals = GraphValues::default();
        vals.data[0] = Some(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let def = GraphDef {
            graph_type: GraphType::Bar,
            features: GraphFeatures {
                orientation: Orientation::Horizontal,
                ..Default::default()
            },
            ..Default::default()
        };
        render(&def, &vals, area, &mut buf);
        assert!(contains(&buf, "█"), "no █ full-block in horizontal bars");
        assert!(
            contains(&buf, "▌"),
            "no ▌ half-block at the right end of a partial bar"
        );
        // Vertical baseline column from frame's left edge or the
        // horizontal renderer's own `│` baseline.
        assert!(contains(&buf, "│"), "no vertical baseline / frame edge");
    }

    #[test]
    fn line_renders_dot_and_baseline() {
        let buf = render_to(GraphType::Line, vec![1.0, 5.0, 3.0, 7.0], 40, 10);
        assert!(contains(&buf, "•"), "no • in line chart");
        assert!(contains(&buf, "─"), "no baseline");
    }

    #[test]
    fn empty_values_shows_placeholder() {
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        let vals = GraphValues::default();
        render(&GraphDef::default(), &vals, area, &mut buf);
        assert!(contains(&buf, "No graph ranges set"));
    }

    #[test]
    fn unimplemented_type_shows_placeholder() {
        let buf = render_to(GraphType::Pie, vec![1.0, 2.0, 3.0], 80, 10);
        assert!(contains(&buf, "Pie"));
        assert!(contains(&buf, "later slice"));
    }

    #[test]
    fn tiny_area_is_safe() {
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        let mut vals = GraphValues::default();
        vals.data[0] = Some(vec![1.0, 2.0, 3.0]);
        render(
            &GraphDef {
                graph_type: GraphType::Bar,
                ..Default::default()
            },
            &vals,
            area,
            &mut buf,
        );
        // No panic = pass. Buffer may be empty.
    }

    #[test]
    fn bar_bar_height_monotonic_with_value() {
        let buf = render_to(GraphType::Bar, vec![1.0, 10.0], 20, 12);
        // Count `█` cells by column. The 10.0 bar should be taller
        // than the 1.0 bar.
        let mut heights = [0u16; 20];
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol() == "█" {
                    heights[x as usize] += 1;
                }
            }
        }
        let small = heights
            .iter()
            .filter(|&&h| h > 0)
            .copied()
            .min()
            .unwrap_or(0);
        let big = heights.iter().copied().max().unwrap_or(0);
        assert!(
            big > small,
            "bigger value should produce taller bar ({big} vs {small})"
        );
    }
}
