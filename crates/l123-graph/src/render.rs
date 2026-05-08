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
///
/// `data_label_text` carries the cell text bound to each per-series
/// `/Graph Options Data-Labels {A-F}` range. Parallel to `data`;
/// only set when the corresponding range is bound. The placement
/// (Above/Below/etc.) lives on `GraphOptions::data_labels_placement`
/// alongside the range itself.
#[derive(Clone, Debug, Default)]
pub struct GraphValues {
    pub x: Option<Vec<f64>>,
    pub x_labels: Option<Vec<String>>,
    pub data: [Option<Vec<f64>>; 6],
    pub data_label_text: [Option<Vec<String>>; 6],
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

    /// Same as [`first_series`](Self::first_series) but also returns
    /// the slot index (0..=5 for A..F) so callers can pull the
    /// matching `data_label_text[slot]` and any other per-slot state.
    pub fn first_series_with_slot(&self) -> Option<(usize, &[f64])> {
        self.data
            .iter()
            .enumerate()
            .find_map(|(i, opt)| opt.as_deref().map(|s| (i, s)))
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
                render_bar(def, vals, inner, buf, def.features.drop_shadow)
            }
            (crate::Orientation::Vertical, true) => {
                render_bar_stacked(def, vals, inner, buf, def.features.percent)
            }
            (crate::Orientation::Horizontal, _) => render_bar_horizontal(def, vals, inner, buf),
        },
        GraphType::Stack => render_bar_stacked(def, vals, inner, buf, def.features.percent),
        GraphType::Line => render_line(def, vals, inner, buf),
        GraphType::Pie => render_pie(vals, inner, buf),
        GraphType::XY => render_xy(def, vals, inner, buf),
        GraphType::Mixed => render_mixed(def, vals, inner, buf),
        GraphType::HLCO => render_hlco(def, vals, inner, buf),
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

    // Inner y-axis line: one column reserved just inside the Left
    // edge (or at the very left when frame.left is off). Same row
    // range as the outer Left edge so the two read as concentric
    // verticals.
    let inset_top = if frame.top { 1 } else { 0 };
    let inset_bottom = if frame.bottom { 1 } else { 0 };
    let inset_left_outer = if frame.left { 1 } else { 0 };
    let inset_right = if frame.right { 1 } else { 0 };
    if frame.y_axis && area.width > inset_left_outer {
        let yax_x = left_x + inset_left_outer;
        for y in (top_y + 1)..bottom_y {
            buf[(yax_x, y)].set_symbol("│");
            buf[(yax_x, y)].set_style(style);
        }
    }

    // Compute inner rectangle. The inner Y-axis line consumes one
    // extra left column beyond the outer Left edge.
    let inset_left = inset_left_outer + if frame.y_axis { 1 } else { 0 };
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
///
/// When `data_labels[slot]` is bound for a populated series, each
/// of that series' bars gets its label cell text painted at the
/// placement offset relative to the bar's top — same idiom as
/// the line renderer.
fn render_bar(
    def: &GraphDef,
    vals: &GraphValues,
    area: Rect,
    buf: &mut Buffer,
    drop_shadow: bool,
) {
    const GLYPHS: [&str; 4] = ["█", "▓", "▒", "░"];

    // Pair each populated series with its original A..F slot so we
    // can pull `data_label_text[slot]` and `data_labels_placement[slot]`.
    let series_with_slot: Vec<(usize, &[f64])> = vals
        .data
        .iter()
        .enumerate()
        .filter_map(|(slot, o)| o.as_deref().map(|s| (slot, s)))
        .collect();
    let series: Vec<&[f64]> = series_with_slot.iter().map(|(_, s)| *s).collect();
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
            // Per-bar data label, if bound for this slot. Anchor at
            // the bar's top cell (the half-block when present, the
            // topmost full row otherwise) and let `paint_data_label`
            // apply the placement offset.
            let (slot, _) = series_with_slot[si];
            if let Some(label) = vals
                .data_label_text
                .get(slot)
                .and_then(|opt| opt.as_deref())
                .and_then(|labels| labels.get(i).filter(|s| !s.is_empty()))
            {
                let bar_top_y =
                    plot_bottom.saturating_sub(full_rows + u16::from(has_half));
                let center_x = x_start + bar_width / 2;
                let placement = def.options.data_labels_placement[slot];
                paint_data_label(label, center_x, bar_top_y, placement, area, buf);
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
///
/// When the matching slot's data labels are bound, each segment's
/// cell text is painted at the placement offset relative to the
/// segment's top — same idiom as the Bar / Line renderers.
fn render_bar_stacked(
    def: &GraphDef,
    vals: &GraphValues,
    area: Rect,
    buf: &mut Buffer,
    percent: bool,
) {
    const GLYPHS: [&str; 4] = ["█", "▓", "▒", "░"];

    // Pair each populated series with its original A..F slot so
    // `data_label_text[slot]` and `data_labels_placement[slot]`
    // line up.
    let series_with_slot: Vec<(usize, &[f64])> = vals
        .data
        .iter()
        .enumerate()
        .filter_map(|(slot, o)| o.as_deref().map(|s| (slot, s)))
        .collect();
    let series: Vec<&[f64]> = series_with_slot.iter().map(|(_, s)| *s).collect();
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
            // Per-segment data label, if bound for this slot. The
            // segment's top y is `plot_bottom - row_offset` after the
            // row_offset increment above.
            let (slot, _) = series_with_slot[si];
            if let Some(label) = vals
                .data_label_text
                .get(slot)
                .and_then(|opt| opt.as_deref())
                .and_then(|labels| labels.get(i).filter(|s| !s.is_empty()))
            {
                let segment_top_y = plot_bottom.saturating_sub(row_offset);
                let center_x = x0 + bar_width / 2;
                let placement = def.options.data_labels_placement[slot];
                paint_data_label(label, center_x, segment_top_y, placement, area, buf);
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
fn render_bar_horizontal(def: &GraphDef, vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some((slot, series)) = vals.first_series_with_slot() else {
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
        // Per-bar data label, when bound for this slot. Anchor at
        // the bar's right end (just past the last filled cell) and
        // the bar's vertical centre. Right placement reads as "to
        // the right of the bar"; Above / Below land one row above
        // or below; Center / Left fall inside the bar interior.
        if let Some(label) = vals
            .data_label_text
            .get(slot)
            .and_then(|opt| opt.as_deref())
            .and_then(|labels| labels.get(i).filter(|s| !s.is_empty()))
        {
            let bar_end_x = plot_left + full_cols + u16::from(has_half);
            let center_y = y0 + bar_height / 2;
            let placement = def.options.data_labels_placement[slot];
            paint_data_label(label, bar_end_x, center_y, placement, area, buf);
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
/// of how many samples there are. Y bounds default to the A series'
/// own min/max; `/Graph Options Scale Y Manual` with Lower / Upper
/// overrides them via `ScaleAxis::apply`.
///
/// When the matching slot's data labels are bound, each label's
/// cell text is painted near its `•` at the placement offset
/// (Center / Left / Above / Right / Below).
fn render_line(def: &GraphDef, vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some((slot, series)) = vals.first_series_with_slot() else {
        write_centered(area, buf, "No numeric A-series values to plot.");
        return;
    };
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 || series.is_empty() {
        return;
    }
    let (data_min, data_max) = series
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    let (data_min, data_max) =
        if !data_min.is_finite() || !data_max.is_finite() || data_min == data_max {
            (0.0, 1.0)
        } else {
            (data_min, data_max)
        };
    let (min, max) = def.options.scale_y.apply(data_min, data_max);
    let (min, max) = if min >= max { (min, min + 1.0) } else { (min, max) };
    let style = Style::default().fg(Color::Cyan);
    let span = max - min;
    let n = series.len().max(1);
    let denom = (n - 1).max(1) as f64;
    for (i, v) in series.iter().copied().enumerate() {
        if !v.is_finite() {
            continue;
        }
        // Clip to the effective Y window so manual Lower/Upper
        // actually constrain the plot. With Auto bounds this is a
        // no-op since min/max came from the data extent.
        if v < min || v > max {
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
        // Per-point data label, if bound for this slot. Painted
        // after the dot so it sits over the line glyph for
        // Center placement.
        if let Some(label) = vals
            .data_label_text
            .get(slot)
            .and_then(|opt| opt.as_deref())
            .and_then(|labels| labels.get(i).filter(|s| !s.is_empty()))
        {
            let placement = def.options.data_labels_placement[slot];
            paint_data_label(label, x, y, placement, area, buf);
        }
    }
    for x in area.left()..area.right() {
        let cell = &mut buf[(x, plot_bottom)];
        cell.set_symbol("─");
        cell.set_style(Style::default().fg(Color::Gray));
    }
}

/// Paint a data-label string near `(anchor_x, anchor_y)` according
/// to `placement`. Respects `area` bounds — labels that would spill
/// outside are clipped (truncated for Right/Center, repositioned
/// only when fully off-area for Left).
fn paint_data_label(
    text: &str,
    anchor_x: u16,
    anchor_y: u16,
    placement: crate::DataLabelPlacement,
    area: Rect,
    buf: &mut Buffer,
) {
    use crate::DataLabelPlacement::*;
    let style = Style::default().fg(Color::White);
    let len = text.chars().count() as u16;
    let (lx, ly) = match placement {
        Above => {
            if anchor_y == area.top() {
                return;
            }
            (anchor_x.saturating_sub(len / 2), anchor_y - 1)
        }
        Below => {
            if anchor_y + 1 >= area.bottom() {
                return;
            }
            (anchor_x.saturating_sub(len / 2), anchor_y + 1)
        }
        Center => (anchor_x.saturating_sub(len / 2), anchor_y),
        Left => {
            if anchor_x < len + 1 {
                return;
            }
            (anchor_x - len - 1, anchor_y)
        }
        Right => (anchor_x.saturating_add(1), anchor_y),
    };
    if ly < area.top() || ly >= area.bottom() {
        return;
    }
    let max_len = area.right().saturating_sub(lx);
    if max_len == 0 {
        return;
    }
    let truncated: String = text.chars().take(max_len as usize).collect();
    buf.set_string(lx, ly, truncated, style);
}

/// HLCO (High-Low-Close-Open) chart, terminal flavor. Each x
/// position renders a vertical `│` line spanning B[i] (low) to
/// A[i] (high), with `─` ticks pointing right at C[i] (close) and
/// left at D[i] (open). Series mapping matches the raster
/// `draw_hlco` convention and Reference p. 2-156.
///
/// All four series share a common Y extent computed across whatever
/// series are populated and finite — different from the Bar/Line
/// renderers which use the A series' own extent. Falls back to a
/// centered hint when A (high) is unset or no series has finite
/// values.
fn render_hlco(def: &GraphDef, vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some(high) = vals.data[0].as_deref() else {
        write_centered(area, buf, "HLCO needs series A (high).");
        return;
    };
    let low = vals.data[1].as_deref().unwrap_or(&[]);
    let close = vals.data[2].as_deref().unwrap_or(&[]);
    let open = vals.data[3].as_deref().unwrap_or(&[]);
    let n = high.len();
    if n == 0 {
        write_centered(area, buf, "HLCO needs series A (high).");
        return;
    }
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 || area.width < 2 {
        return;
    }

    let mut data_lo = f64::INFINITY;
    let mut data_hi = f64::NEG_INFINITY;
    for s in [high, low, close, open] {
        for &v in s {
            if v.is_finite() {
                data_lo = data_lo.min(v);
                data_hi = data_hi.max(v);
            }
        }
    }
    if !data_lo.is_finite() || !data_hi.is_finite() {
        write_centered(area, buf, "HLCO has no numeric values.");
        return;
    }
    let (y_lo, y_hi) = def.options.scale_y.apply(data_lo, data_hi);
    let (y_lo, y_hi) = if y_lo >= y_hi {
        (y_lo, y_lo + 1.0)
    } else {
        (y_lo, y_hi)
    };
    let span = if (y_hi - y_lo).abs() < f64::EPSILON {
        1.0
    } else {
        y_hi - y_lo
    };
    let denom = (n.saturating_sub(1)).max(1) as f64;
    let bar_style = Style::default().fg(Color::Gray);
    let close_style = Style::default().fg(Color::Green);
    let open_style = Style::default().fg(Color::Red);

    let to_y = |v: f64| -> u16 {
        let frac = (v - y_lo) / span;
        let from_bottom = (frac * (plot_height as f64 - 1.0)).round() as u16;
        plot_bottom.saturating_sub(1 + from_bottom)
    };
    for (i, &h) in high.iter().enumerate().take(n) {
        let l = low.get(i).copied().unwrap_or(f64::NAN);
        if !h.is_finite() || !l.is_finite() {
            continue;
        }
        let frac_x = if n == 1 { 0.5 } else { i as f64 / denom };
        let bx = area.left() + (frac_x * (area.width as f64 - 1.0)).round() as u16;
        if bx >= area.right() {
            continue;
        }
        // Clip H/L into the effective Y window so manual bounds
        // actually constrain the visible bar.
        let bar_top = h.max(l).min(y_hi);
        let bar_bot = h.min(l).max(y_lo);
        if bar_top < bar_bot {
            continue;
        }
        let by_high = to_y(bar_top);
        let by_low = to_y(bar_bot);
        for y in by_high..=by_low {
            if y < area.top() || y >= area.bottom() {
                continue;
            }
            let cell = &mut buf[(bx, y)];
            cell.set_symbol("│");
            cell.set_style(bar_style);
        }
        let c = close.get(i).copied().unwrap_or(f64::NAN);
        if c.is_finite() && c >= y_lo && c <= y_hi {
            let by = to_y(c);
            let cx = bx.saturating_add(1);
            if cx < area.right() && by >= area.top() && by < area.bottom() {
                let cell = &mut buf[(cx, by)];
                cell.set_symbol("─");
                cell.set_style(close_style);
            }
        }
        let o = open.get(i).copied().unwrap_or(f64::NAN);
        if o.is_finite() && o >= y_lo && o <= y_hi && bx > area.left() {
            let by = to_y(o);
            let ox = bx - 1;
            if by >= area.top() && by < area.bottom() {
                let cell = &mut buf[(ox, by)];
                cell.set_symbol("─");
                cell.set_style(open_style);
            }
        }
        // Per-bar data label, when bound for slot 0 (the High
        // series, which is HLCO's primary anchor for labels).
        if let Some(label) = vals.data_label_text[0]
            .as_deref()
            .and_then(|labels| labels.get(i).filter(|s| !s.is_empty()))
        {
            let bar_top_y = to_y(bar_top);
            let placement = def.options.data_labels_placement[0];
            paint_data_label(label, bx, bar_top_y, placement, area, buf);
        }
    }
    for x in area.left()..area.right() {
        let cell = &mut buf[(x, plot_bottom)];
        cell.set_symbol("─");
        cell.set_style(Style::default().fg(Color::Gray));
    }
}

/// Mixed graph, terminal flavor: A series renders as bars and B
/// series renders as a dot-line over the same plot area, matching
/// the raster `draw_mixed` convention. Each layer scales to its
/// own extent — the original 1-2-3 R3.4a Mixed graph optionally
/// maps B to the secondary (2Y) axis, which is the same effect.
///
/// When only A is set this collapses to a plain Bar; when only B
/// is set, to a plain Line. When neither is set, prints a centered
/// hint so the user knows to bind a series.
fn render_mixed(def: &GraphDef, vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let a = vals.data[0].clone();
    let b = vals.data[1].clone();
    if a.is_none() && b.is_none() {
        write_centered(area, buf, "Mixed needs at least one of A or B.");
        return;
    }
    if a.is_some() {
        let bars = GraphValues {
            data: [a, None, None, None, None, None],
            ..Default::default()
        };
        render_bar(def, &bars, area, buf, false);
    }
    if b.is_some() {
        let line = GraphValues {
            data: [b, None, None, None, None, None],
            ..Default::default()
        };
        render_line(def, &line, area, buf);
    }
}

/// XY scatter, terminal flavor: one `•` per (X[i], A[i]) pair,
/// placed at the proportional position of x within the X-range
/// extent and y within the A-range extent. The distinguishing
/// feature vs. Line is that x positions come from the X range, not
/// from a uniform sample-index grid — points can repeat, cluster,
/// or land out of input order.
///
/// Falls back to a centered error message when X is unset or has no
/// finite values; when A is unset, says so. When all (x, y) pairs
/// collapse to a single distinct x or y the corresponding axis span
/// becomes a unit interval so the points still render at the
/// midline.
fn render_xy(def: &GraphDef, vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    let Some(xs) = vals.x.as_deref() else {
        write_centered(area, buf, "XY graphs need an X range.");
        return;
    };
    let Some(ys) = vals.data[0].as_deref() else {
        write_centered(area, buf, "XY graphs need series A.");
        return;
    };
    let n = xs.len().min(ys.len());
    if n == 0 {
        write_centered(area, buf, "XY needs aligned X and A values.");
        return;
    }
    let plot_top = area.top();
    let plot_bottom = area.bottom().saturating_sub(1);
    let plot_height = plot_bottom.saturating_sub(plot_top);
    if plot_height < 2 || area.width < 2 {
        return;
    }

    // Keep the original index alongside each filtered pair so the
    // matching `data_label_text[0][orig_i]` lines up after the
    // non-finite filter.
    let pairs: Vec<(usize, f64, f64)> = (0..n)
        .filter(|i| xs[*i].is_finite() && ys[*i].is_finite())
        .map(|i| (i, xs[i], ys[i]))
        .collect();
    if pairs.is_empty() {
        write_centered(area, buf, "XY needs finite X and A values.");
        return;
    }
    let (data_x_min, data_x_max) = pairs.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(lo, hi), (_, x, _)| (lo.min(*x), hi.max(*x)),
    );
    let (data_y_min, data_y_max) = pairs.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(lo, hi), (_, _, y)| (lo.min(*y), hi.max(*y)),
    );
    let (x_min, x_max) = def.options.scale_x.apply(data_x_min, data_x_max);
    let (y_min, y_max) = def.options.scale_y.apply(data_y_min, data_y_max);
    let x_span = if (x_max - x_min).abs() < f64::EPSILON {
        1.0
    } else {
        x_max - x_min
    };
    let y_span = if (y_max - y_min).abs() < f64::EPSILON {
        1.0
    } else {
        y_max - y_min
    };
    let style = Style::default().fg(Color::Cyan);
    let label_placement = def.options.data_labels_placement[0];
    let labels = vals.data_label_text[0].as_deref();
    for (orig_i, x, y) in pairs {
        // Clip to the effective X / Y windows so manual bounds
        // actually constrain the plot.
        if x < x_min || x > x_max || y < y_min || y > y_max {
            continue;
        }
        let frac_x = (x - x_min) / x_span;
        let frac_y = (y - y_min) / y_span;
        let bx = area.left() + (frac_x * (area.width as f64 - 1.0)).round() as u16;
        let y_from_bottom = (frac_y * (plot_height as f64 - 1.0)).round() as u16;
        let by = plot_bottom.saturating_sub(1 + y_from_bottom);
        if bx < area.right() && by >= area.top() && by < area.bottom() {
            let cell = &mut buf[(bx, by)];
            cell.set_symbol("•");
            cell.set_style(style);
        }
        // Per-dot data label, when bound for slot 0.
        if let Some(label) = labels
            .and_then(|labels| labels.get(orig_i).filter(|s| !s.is_empty()))
        {
            paint_data_label(label, bx, by, label_placement, area, buf);
        }
    }
    for x in area.left()..area.right() {
        let cell = &mut buf[(x, plot_bottom)];
        cell.set_symbol("─");
        cell.set_style(Style::default().fg(Color::Gray));
    }
}

/// Pie chart, terminal flavor: a single horizontal proportional bar
/// where each wedge is a contiguous run of cells, glyph per wedge in
/// the same A=`█` B=`▓` C=`▒` D=`░` palette as the bar/stack
/// renderers. A labels row underneath shows each wedge's text — the
/// X-range cell text per wedge when bound, else a 1-based positional
/// index. The bar is centered vertically in `area`.
///
/// Matches the data-flow contract that `draw_pie` (raster) uses:
/// non-finite or non-positive A values are dropped so that the
/// surviving wedges keep their original X-index alignment for
/// labels. When no positive values remain, falls back to a centered
/// "Pie needs positive values." message.
fn render_pie(vals: &GraphValues, area: Rect, buf: &mut Buffer) {
    const GLYPHS: [&str; 4] = ["█", "▓", "▒", "░"];

    let Some(a) = vals.data[0].as_deref() else {
        write_centered(area, buf, "Set /Graph A to plot a pie.");
        return;
    };
    let pairs: Vec<(usize, f64)> = a
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, v)| v.is_finite() && *v > 0.0)
        .collect();
    if pairs.is_empty() {
        write_centered(area, buf, "Pie needs positive values.");
        return;
    }
    let total: f64 = pairs.iter().map(|(_, v)| *v).sum();
    if total <= 0.0 {
        write_centered(area, buf, "Pie needs positive values.");
        return;
    }
    let plot_width = area.width;
    if plot_width < 4 || area.height < 3 {
        return;
    }
    let bar_y = area.top() + area.height / 2;
    let label_y = bar_y.saturating_add(1);
    let style = Style::default().fg(Color::Cyan);

    // Walk wedges left to right. Use the cumulative share (not the
    // wedge's own share) to compute each end-x so rounding error
    // never starves the next wedge — the alternative leaves a thin
    // wedge with one cell when an earlier wedge rounds upward.
    let mut cursor: u16 = area.left();
    let mut cumulative: f64 = 0.0;
    let bar_right = area.right();
    let labels = vals.x_labels.as_deref();
    for (wedge_i, (orig_i, v)) in pairs.iter().enumerate() {
        let last = wedge_i + 1 == pairs.len();
        cumulative += *v;
        let target_end = if last {
            bar_right
        } else {
            let frac = cumulative / total;
            area.left() + (frac * plot_width as f64).round() as u16
        };
        let end = target_end.max(cursor.saturating_add(1)).min(bar_right);
        let glyph = GLYPHS[wedge_i % GLYPHS.len()];
        for x in cursor..end {
            let cell = &mut buf[(x, bar_y)];
            cell.set_symbol(glyph);
            cell.set_style(style);
        }
        // Labels: write each wedge's label centered under its run,
        // truncated to fit. Labels overlap when wedges are tiny;
        // that's the same trade-off the raster Pie hits.
        if label_y < area.bottom() {
            let label = wedge_label_unicode(labels, *orig_i, wedge_i);
            let run_w = end.saturating_sub(cursor);
            if run_w > 0 {
                let truncated: String = label.chars().take(run_w as usize).collect();
                let len = truncated.chars().count() as u16;
                let pad = run_w.saturating_sub(len) / 2;
                let lx = cursor + pad;
                buf.set_string(lx, label_y, truncated, style);
            }
        }
        cursor = end;
        if cursor >= bar_right {
            break;
        }
    }
}

/// Mirror of `raster::wedge_label`: prefers the cell-text label for
/// the original X position, falls back to a 1-based positional
/// index when the cell is blank. Kept private to render.rs because
/// it's intentionally the same contract as the raster path so the
/// two backends agree on every wedge's label.
fn wedge_label_unicode(x_labels: Option<&[String]>, orig_i: usize, wedge_i: usize) -> String {
    if let Some(labels) = x_labels {
        if let Some(s) = labels.get(orig_i) {
            if !s.trim().is_empty() {
                return s.clone();
            }
        }
    }
    format!("{}", wedge_i + 1)
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

    #[test]
    fn xy_uses_x_values_for_horizontal_position() {
        // X = [0, 10, 100], Y = [50, 50, 50]. With Line semantics
        // the three dots would be evenly spaced. With XY semantics
        // they should cluster — points at x=0 and x=10 are within
        // ~10% of total span, while x=100 sits at the right edge.
        let area = Rect::new(0, 0, 60, 16);
        let mut buf = Buffer::empty(area);
        let vals = GraphValues {
            x: Some(vec![0.0, 10.0, 100.0]),
            data: [
                Some(vec![50.0, 50.0, 50.0]),
                None,
                None,
                None,
                None,
                None,
            ],
            ..Default::default()
        };
        let def = GraphDef {
            graph_type: GraphType::XY,
            ..Default::default()
        };
        render(&def, &vals, area, &mut buf);
        let mut dot_xs: Vec<u16> = (0..buf.area.width)
            .filter(|x| {
                (0..buf.area.height).any(|y| buf[(*x, y)].symbol() == "•")
            })
            .collect();
        dot_xs.sort_unstable();
        assert_eq!(dot_xs.len(), 3, "expected 3 distinct dot columns; got {dot_xs:?}");
        // First two cluster near the left, third lands far to the
        // right — the gap between the second and third dot is much
        // larger than between the first and second.
        let near = dot_xs[1] - dot_xs[0];
        let far = dot_xs[2] - dot_xs[1];
        assert!(
            far > near * 3,
            "XY should cluster near-x dots and stretch far ones (near={near}, far={far})"
        );
    }

    #[test]
    fn hlco_draws_open_left_close_right_of_bar() {
        // Single x-position so the bar's column is unambiguous.
        // High=10, Low=2, Close=8, Open=4. Open tick should land
        // one column LEFT of the bar's column; close one column RIGHT.
        let area = Rect::new(0, 0, 30, 14);
        let mut buf = Buffer::empty(area);
        let vals = GraphValues {
            x: None,
            data: [
                Some(vec![10.0]),
                Some(vec![2.0]),
                Some(vec![8.0]),
                Some(vec![4.0]),
                None,
                None,
            ],
            ..Default::default()
        };
        let def = GraphDef {
            graph_type: GraphType::HLCO,
            ..Default::default()
        };
        render(&def, &vals, area, &mut buf);
        // Find the bar column: an INTERIOR column with stacked │.
        // The frame's left/right edges also contain │, so skip x=0
        // and x=width-1 to isolate the data bar.
        let bar_col = (1..buf.area.width - 1).find(|x| {
            (0..buf.area.height)
                .filter(|y| buf[(*x, *y)].symbol() == "│")
                .count()
                >= 2
        });
        let bar_col = bar_col.expect("expected an interior column with stacked │ glyphs for the H-L bar");
        // A `─` should appear at the column immediately to the right
        // (close) and immediately to the left (open) — distinct from
        // the baseline row, where `─` runs across the whole width.
        let baseline = area.bottom() - 2;
        let has_tick_above_baseline = |x: u16| -> bool {
            (0..baseline).any(|y| buf[(x, y)].symbol() == "─")
        };
        assert!(bar_col > 0, "bar column at left edge — no room for open tick");
        assert!(
            has_tick_above_baseline(bar_col + 1),
            "expected close ─ tick at column {} (right of bar at {bar_col})",
            bar_col + 1
        );
        assert!(
            has_tick_above_baseline(bar_col - 1),
            "expected open ─ tick at column {} (left of bar at {bar_col})",
            bar_col - 1
        );
    }

    #[test]
    fn line_honors_manual_y_upper_bound() {
        // Data [1,2,3,4]. Without manual: the topmost • sits at the
        // first plot row (max value pinned to top). With Manual
        // Upper=100: the topmost • should land far below the top
        // because 4/100 = 4% of the plot height.
        let area = Rect::new(0, 0, 40, 14);
        let topmost_dot_y = |def: &GraphDef| -> u16 {
            let mut buf = Buffer::empty(area);
            let vals = GraphValues {
                data: [Some(vec![1.0, 2.0, 3.0, 4.0]), None, None, None, None, None],
                ..Default::default()
            };
            render(def, &vals, area, &mut buf);
            (0..buf.area.height)
                .find(|y| (0..buf.area.width).any(|x| buf[(x, *y)].symbol() == "•"))
                .expect("expected at least one • in line plot")
        };

        let auto_def = GraphDef {
            graph_type: GraphType::Line,
            ..Default::default()
        };
        let auto_top = topmost_dot_y(&auto_def);

        let mut manual_def = auto_def.clone();
        manual_def.options.scale_y.mode = crate::ScaleMode::Manual;
        manual_def.options.scale_y.upper = Some(100.0);
        let manual_top = topmost_dot_y(&manual_def);

        assert!(
            manual_top > auto_top + 4,
            "Manual Upper=100 should push the topmost dot well below the auto top \
             (auto={auto_top}, manual={manual_top})"
        );
    }
}
