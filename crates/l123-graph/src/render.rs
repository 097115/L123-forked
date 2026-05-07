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
///
/// `x_labels` carries the *display text* of each X-range cell so
/// renderers that label categorical positions (Pie wedges, eventually
/// Bar/Line tick labels) can use the user's strings instead of
/// positional indices. Parallel to `x` and only set when X is bound.
#[derive(Clone, Debug, Default)]
pub struct GraphValues {
    pub x: Option<Vec<f64>>,
    pub x_labels: Option<Vec<String>>,
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
    let with_notes = reserve_note_row(with_titles, &def.options.titles, buf);
    let with_legend = reserve_legend_row(with_notes, &def.options.legend, buf);
    let after_table = if def.features.table {
        reserve_value_table_rows(with_legend, vals, buf)
    } else {
        with_legend
    };
    // Y-Axis / 2Y-Axis titles sit OUTSIDE the frame, in one-column
    // vertical strips on the left and right of the plot. Carve those
    // before the frame so the frame stays a clean rectangle.
    let plot_area = reserve_axis_title_columns(after_table, &def.options.titles, buf);
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
        GraphType::Bar => match (def.features.orientation, def.features.stacked) {
            (crate::Orientation::Vertical, false) => {
                render_bar(vals, inner, buf, def.features.drop_shadow)
            }
            (crate::Orientation::Vertical, true) => {
                render_bar_stacked(vals, inner, buf, def.features.percent)
            }
            (crate::Orientation::Horizontal, _) => render_bar_horizontal(vals, inner, buf),
        },
        GraphType::Stack => render_bar_stacked(vals, inner, buf, def.features.percent),
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

/// Paint Note (bottom-left) and Other-Note (bottom-right) on a
/// shared row at the bottom of `area`, then return the area shrunk
/// by one row when at least one is set. Reference p. 2-216 places
/// these as the bottom-most footnotes; in our terminal layout we
/// position the note row directly above the X-Axis title (which
/// reserve_title_rows already carved out at `area.bottom() - 1` —
/// or, when no X-Axis title, at the very bottom).
///
/// When both Note and Other-Note share the row and would overlap
/// at the chosen widths, Note wins on the left and Other-Note is
/// truncated; that's a graceful fallback for narrow terminals.
fn reserve_note_row(area: Rect, titles: &crate::Titles, buf: &mut Buffer) -> Rect {
    let note = titles.note.as_deref();
    let other = titles.other_note.as_deref();
    if note.is_none() && other.is_none() {
        return area;
    }
    if area.height < 4 {
        return area;
    }
    let style = Style::default().fg(Color::White);
    let y = area.bottom().saturating_sub(1);
    if let Some(text) = note {
        let display: String = text
            .chars()
            .take(area.width.saturating_sub(2) as usize)
            .collect();
        buf.set_string(area.left() + 1, y, display, style);
    }
    if let Some(text) = other {
        let len = text.chars().count() as u16;
        let max_len = area.width.saturating_sub(2);
        let display: String = if len > max_len {
            text.chars().take(max_len as usize).collect()
        } else {
            text.to_owned()
        };
        let display_len = display.chars().count() as u16;
        let x = area.left() + area.width.saturating_sub(display_len + 1);
        buf.set_string(x, y, display, style);
    }
    Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1))
}

/// Paint a small value table at the bottom of `area`, one row per
/// populated A..F series, listing each series' values across X
/// positions. Returns the area shrunk upward by however many rows
/// the table consumed. Per Reference p. 2-198, this surfaces under
/// `/Graph Type Features Table Yes` for line / bar / stacked bar /
/// mixed graphs.
///
/// No-op when no series is populated, when `n` (max series length)
/// is zero, or when there isn't enough vertical or horizontal room.
/// Each value gets a fixed-width column so columns line up.
fn reserve_value_table_rows(area: Rect, vals: &GraphValues, buf: &mut Buffer) -> Rect {
    let series: Vec<(usize, &[f64])> = vals
        .data
        .iter()
        .enumerate()
        .filter_map(|(i, opt)| opt.as_deref().map(|s| (i, s)))
        .collect();
    if series.is_empty() {
        return area;
    }
    let rows_needed = series.len() as u16;
    if rows_needed + 3 >= area.height || area.width < 16 {
        return area;
    }
    const LABEL_WIDTH: u16 = 4; // "A:  "
    const COL_WIDTH: u16 = 7;
    let style = Style::default().fg(Color::White);
    let top = area.bottom().saturating_sub(rows_needed);
    for (row_i, (slot, s)) in series.iter().enumerate() {
        let y = top + row_i as u16;
        let letter = (b'A' + *slot as u8) as char;
        buf.set_string(area.left() + 1, y, format!("{letter}: "), style);
        for (col, &v) in s.iter().enumerate() {
            let x = area.left() + 1 + LABEL_WIDTH + (col as u16) * COL_WIDTH;
            if x + COL_WIDTH > area.right() {
                break;
            }
            let text = if v.is_finite() {
                format!("{:>w$}", strip_trailing_zero(v), w = COL_WIDTH as usize)
            } else {
                format!("{:>w$}", "-", w = COL_WIDTH as usize)
            };
            buf.set_string(x, y, text, style);
        }
    }
    Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(rows_needed),
    )
}

/// Render `v` without a trailing `.0` when it's an integer; up to
/// two decimal places otherwise. Keeps the table tidy.
fn strip_trailing_zero(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
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

/// Paint Y-Axis (left) and 2Y-Axis (right) titles as one-column-wide
/// vertical text and return the area shrunk inward by one column on
/// each side that carries a title. Each character of the title goes
/// in its own row, top-to-bottom, vertically centered within the
/// available column. Truncated when the title is longer than the
/// column height. No-op when neither title is set.
fn reserve_axis_title_columns(area: Rect, titles: &crate::Titles, buf: &mut Buffer) -> Rect {
    let left = titles.y_axis.as_deref();
    let right = titles.two_y_axis.as_deref();
    if left.is_none() && right.is_none() {
        return area;
    }
    if area.width < 4 {
        return area;
    }
    let style = Style::default().fg(Color::White);
    let mut x = area.left();
    let mut width = area.width;
    if let Some(text) = left {
        write_centered_column(area, area.left(), buf, text, style);
        x = x.saturating_add(1);
        width = width.saturating_sub(1);
    }
    if let Some(text) = right {
        let col = area.right().saturating_sub(1);
        write_centered_column(area, col, buf, text, style);
        width = width.saturating_sub(1);
    }
    Rect::new(x, area.y, width, area.height)
}

fn write_centered_column(area: Rect, x: u16, buf: &mut Buffer, msg: &str, style: Style) {
    let chars: Vec<char> = msg.chars().collect();
    let max_len = area.height as usize;
    let take = chars.len().min(max_len);
    let y0 = area.top() + ((area.height as usize - take) / 2) as u16;
    for (i, ch) in chars.iter().take(take).enumerate() {
        let y = y0 + i as u16;
        if y >= area.bottom() {
            break;
        }
        let cell = &mut buf[(x, y)];
        cell.set_symbol(&ch.to_string());
        cell.set_style(style);
    }
}

/// Paint First / Second / X-Axis titles into `area` and return the
/// remaining rectangle the plot is allowed to occupy. Y-Axis and
/// 2Y-Axis titles are handled separately by
/// [`reserve_axis_title_columns`].
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

/// Vertical bar chart. Every populated A..F slot becomes its own
/// series in a side-by-side cluster at each X position (the default
/// 1-2-3 R3.4a layout for multi-series Bar; Reference p. 2-156).
/// Each series uses a distinct shading glyph so identity reads
/// visually: A=`█`, B=`▓`, C=`▒`, D=`░` (E/F cycle).
///
/// Height resolution is half-row: `▄` (lower half block) caps the
/// partial cell at the top of a bar — same series-agnostic glyph
/// used by the original single-series renderer because mixing two
/// series across one cell can't be represented in a single Buffer
/// cell.
///
/// With only series A populated this collapses to the original
/// single-series Bar layout: one wide bar per X, half-block top.
fn render_bar(vals: &GraphValues, area: Rect, buf: &mut Buffer, drop_shadow: bool) {
    const GLYPHS: [&str; 4] = ["█", "▓", "▒", "░"];

    let series: Vec<&[f64]> = vals.data.iter().filter_map(|o| o.as_deref()).collect();
    if series.is_empty() {
        write_centered(area, buf, "No A..F data to plot.");
        return;
    }
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 {
        return;
    }
    let n = series.iter().map(|s| s.len()).max().unwrap_or(0);
    if n == 0 {
        return;
    }
    // Global max across every populated series — bar heights compare
    // across the whole graph.
    let max = series
        .iter()
        .flat_map(|s| s.iter())
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    let max = if max <= 0.0 || !max.is_finite() {
        1.0
    } else {
        max
    };

    let group_count = n as u16;
    let plot_width = area.width;
    let group_step = (plot_width / group_count).max(1);
    let group_width = group_step.saturating_sub(1).max(1);
    let series_count = series.len() as u16;
    // Bars within a group share the group's width.
    let bar_width = (group_width / series_count).max(1);

    let half = "▄";
    let style = Style::default().fg(Color::Cyan);

    for i in 0..n {
        let group_x0 = area.left() + (i as u16) * group_step;
        for (si, s) in series.iter().enumerate() {
            let v = s.get(i).copied().unwrap_or(f64::NAN);
            if !v.is_finite() || v <= 0.0 {
                continue;
            }
            let x_start = group_x0 + (si as u16) * bar_width;
            let height_halves = ((v / max) * plot_height as f64 * 2.0).round() as u16;
            let full_rows = height_halves / 2;
            let has_half = height_halves % 2 == 1;
            let glyph = GLYPHS[si % GLYPHS.len()];
            for row in 0..full_rows {
                let y = plot_bottom.saturating_sub(1 + row);
                if y < area.top() {
                    break;
                }
                for bx in 0..bar_width {
                    let x = x_start + bx;
                    if x >= area.right() {
                        break;
                    }
                    let cell = &mut buf[(x, y)];
                    cell.set_symbol(glyph);
                    cell.set_style(style);
                }
            }
            if has_half {
                let y = plot_bottom.saturating_sub(1 + full_rows);
                if y >= area.top() {
                    for bx in 0..bar_width {
                        let x = x_start + bx;
                        if x >= area.right() {
                            break;
                        }
                        let cell = &mut buf[(x, y)];
                        cell.set_symbol(half);
                        cell.set_style(style);
                    }
                }
            }
            // Drop-shadow column immediately to the right of the bar
            // for the bar's full height, plus one cell below at the
            // shadow column. Inner-cluster bars get their shadow
            // overdrawn by the next bar — that's the intended 3-D
            // effect.
            if drop_shadow {
                let shadow_x = x_start + bar_width;
                if shadow_x < area.right() {
                    let shadow_style = Style::default().fg(Color::DarkGray);
                    let bar_top = plot_bottom.saturating_sub(full_rows + u16::from(has_half));
                    for y in bar_top..plot_bottom {
                        if y < area.top() {
                            continue;
                        }
                        let cell = &mut buf[(shadow_x, y)];
                        cell.set_symbol("░");
                        cell.set_style(shadow_style);
                    }
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

/// Stacked vertical bar chart. Every populated A..F slot becomes a
/// segment of one column at each X position; segments stack from the
/// bottom up, A first, B above A, etc. Each series uses a distinct
/// shading glyph so series identity reads visually in the unicode
/// view: A=`█`, B=`▓`, C=`▒`, D=`░` (E/F cycle through the same
/// four).
///
/// Used by both `Bar + features.stacked = true` and the dedicated
/// `Stack` graph type. Whole-cell segments only — no half-row
/// blending — because a single Buffer cell can carry only one glyph
/// + style, and mixing two series across one cell can't be
///   represented faithfully.
fn render_bar_stacked(vals: &GraphValues, area: Rect, buf: &mut Buffer, percent: bool) {
    const GLYPHS: [&str; 4] = ["█", "▓", "▒", "░"];

    let series: Vec<&[f64]> = vals.data.iter().filter_map(|o| o.as_deref()).collect();
    if series.is_empty() {
        write_centered(area, buf, "No A..F data to stack.");
        return;
    }
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 {
        return;
    }
    let n = series.iter().map(|s| s.len()).max().unwrap_or(0);
    if n == 0 {
        return;
    }
    // For each X, sum positive finite values across series.
    let mut totals = vec![0f64; n];
    for s in &series {
        for (i, &v) in s.iter().enumerate() {
            if v.is_finite() && v > 0.0 && i < totals.len() {
                totals[i] += v;
            }
        }
    }
    let max_total = totals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let max = if max_total <= 0.0 || !max_total.is_finite() {
        1.0
    } else {
        max_total
    };

    let bar_count = n as u16;
    let plot_width = area.width;
    let step = (plot_width / bar_count).max(1);
    let bar_width = step.saturating_sub(1).max(1);

    let style = Style::default().fg(Color::Cyan);

    for (i, &col_total) in totals.iter().enumerate() {
        let x0 = area.left() + (i as u16) * step;
        // Per-column denominator: in percent mode each column scales
        // by its own total, so every column reaches plot_height; in
        // absolute mode all columns share the global max so column
        // magnitudes are comparable.
        let denom = if percent {
            if col_total > 0.0 {
                col_total
            } else {
                continue;
            }
        } else {
            max
        };
        // Stack from the bottom up.
        let mut row_offset: u16 = 0;
        for (si, s) in series.iter().enumerate() {
            let v = s.get(i).copied().unwrap_or(f64::NAN);
            if !v.is_finite() || v <= 0.0 {
                continue;
            }
            let segment_rows = ((v / denom) * plot_height as f64).round() as u16;
            if segment_rows == 0 {
                continue;
            }
            let glyph = GLYPHS[si % GLYPHS.len()];
            for r in 0..segment_rows {
                let total_rows = row_offset + r;
                if total_rows >= plot_height {
                    break;
                }
                let y = plot_bottom.saturating_sub(1 + total_rows);
                if y < plot_top {
                    break;
                }
                for bx in 0..bar_width {
                    let x = x0 + bx;
                    if x >= area.right() {
                        break;
                    }
                    let cell = &mut buf[(x, y)];
                    cell.set_symbol(glyph);
                    cell.set_style(style);
                }
            }
            row_offset = row_offset.saturating_add(segment_rows);
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
    fn stacked_percent_normalizes_each_column_to_full_height() {
        // A=[1,1], B=[1,2]. Totals=[2,3]. max=3.
        // Without percent: column 0 reaches 2/3 of plot_height; only
        // column 1 (full height) has any cell at the topmost plot row.
        // With percent: both columns reach plot_height; both have
        // cells at the topmost row.
        let area = Rect::new(0, 0, 30, 12);
        let mut vals = GraphValues::default();
        vals.data[0] = Some(vec![1.0, 1.0]);
        vals.data[1] = Some(vec![1.0, 2.0]);

        let count_top_row = |percent: bool| -> usize {
            let mut buf = Buffer::empty(area);
            let def = GraphDef {
                graph_type: GraphType::Bar,
                features: GraphFeatures {
                    stacked: true,
                    percent,
                    ..Default::default()
                },
                ..Default::default()
            };
            render(&def, &vals, area, &mut buf);
            // After title rows (none) + legend (none) + frame (default
            // all-on): inner area starts at y=1 inside the frame.
            // Topmost plot row inside the frame.
            let y = 1;
            (0..area.width)
                .filter(|x| {
                    let s = buf[(*x, y)].symbol();
                    s == "█" || s == "▓" || s == "▒" || s == "░"
                })
                .count()
        };

        let absolute = count_top_row(false);
        let scaled = count_top_row(true);
        assert!(
            scaled > absolute,
            "percent should fill more cells in the top row \
             (absolute={absolute}, percent={scaled})"
        );
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
