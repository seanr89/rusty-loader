//! Pure Rust offline vector PDF 1.7 report generator using PDF Standard 14 fonts.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use reqwest::StatusCode;

use crate::metrics::snapshot::TelemetrySnapshot;
use crate::reporter::export::{write_export_bytes, ExportReport};
use crate::reporter::table::{format_bytes, format_latency, format_transfer_rate};

const PAGE_W: f32 = 612.0; // US Letter width in points
const PAGE_H: f32 = 792.0; // US Letter height in points
const MARGIN: f32 = 36.0;
const CONTENT_W: f32 = PAGE_W - (MARGIN * 2.0); // 540.0 pt

const FONT_REGULAR: Name<'static> = Name(b"F1");
const FONT_BOLD: Name<'static> = Name(b"F2");
const FONT_MONO: Name<'static> = Name(b"F3");

const COLOR_INK: (u8, u8, u8) = (15, 23, 42); // #0f172a
const COLOR_SECONDARY: (u8, u8, u8) = (71, 85, 105); // #475569
const COLOR_MUTED: (u8, u8, u8) = (100, 116, 139); // #64748b
const COLOR_ACCENT: (u8, u8, u8) = (3, 105, 161); // #0369a1
const COLOR_ACCENT_SOFT: (u8, u8, u8) = (224, 242, 254); // #e0f2fe
const COLOR_BORDER: (u8, u8, u8) = (226, 232, 240); // #e2e8f0
const COLOR_CARD_BG: (u8, u8, u8) = (248, 250, 252); // #f8fafc
const COLOR_ROW_ALT: (u8, u8, u8) = (241, 245, 249); // #f1f5f9
const COLOR_WHITE: (u8, u8, u8) = (255, 255, 255);
const COLOR_BANNER_MUTED: (u8, u8, u8) = (203, 213, 225); // #cbd5e1

// Series colors harmonized with the desktop UI charts
const COLOR_SKY: (u8, u8, u8) = (14, 165, 233); // #0ea5e9
const COLOR_INDIGO: (u8, u8, u8) = (99, 102, 241); // #6366f1
const COLOR_AMBER: (u8, u8, u8) = (245, 158, 11); // #f59e0b
const COLOR_EMERALD: (u8, u8, u8) = (16, 185, 129); // #10b981
const COLOR_SKY_LIGHT: (u8, u8, u8) = (56, 189, 248); // #38bdf8
const COLOR_ROSE: (u8, u8, u8) = (244, 63, 94); // #f43f5e
const COLOR_PURPLE: (u8, u8, u8) = (168, 85, 247); // #a855f7
const COLOR_RED: (u8, u8, u8) = (239, 68, 68); // #ef4444
const COLOR_PINK: (u8, u8, u8) = (236, 72, 153); // #ec4899

/// Normalizes UTF-8 text into safe WinAnsi / ASCII bytes for PDF Standard 14 fonts.
pub fn pdf_sanitize_winansi(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' | '\r' | '\t' | '\u{00A0}' => out.push(b' '),
            '—' => out.extend_from_slice(b"--"),
            '–' | '−' | '‐' => out.push(b'-'),
            '‘' | '’' | '‚' => out.push(b'\''),
            '“' | '”' | '„' => out.push(b'"'),
            '…' => out.extend_from_slice(b"..."),
            'µ' | 'μ' => out.push(b'u'),
            '×' => out.push(b'x'),
            '÷' => out.push(b'/'),
            '•' | '·' => out.push(b'*'),
            _ => {
                let code = ch as u32;
                if (0x20..=0x7E).contains(&code) || (0xA1..=0xFF).contains(&code) {
                    out.push(code as u8);
                } else if code < 0x20 || (0x7F..=0xA0).contains(&code) {
                    out.push(b' ');
                } else {
                    out.push(b'?');
                }
            }
        }
    }
    out
}

fn sanitize_ascii_display(input: &str) -> String {
    String::from_utf8_lossy(&pdf_sanitize_winansi(input))
        .chars()
        .collect()
}

fn truncate_str(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let prefix: String = text.chars().take(max_chars - 3).collect();
    format!("{prefix}...")
}

fn wrap_str(text: &str, max_chars_per_line: usize, max_lines: usize) -> Vec<String> {
    if text.is_empty() || max_lines == 0 {
        return vec![String::new()];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut lines = Vec::new();
    let mut idx = 0;

    while idx < chars.len() && lines.len() < max_lines {
        let remaining = chars.len() - idx;
        if lines.len() + 1 == max_lines && remaining > max_chars_per_line {
            let take = max_chars_per_line.saturating_sub(3);
            let mut s: String = chars[idx..idx + take].iter().collect();
            s.push_str("...");
            lines.push(s);
            break;
        }
        let take = remaining.min(max_chars_per_line);
        let chunk: String = chars[idx..idx + take].iter().collect();
        lines.push(chunk);
        idx += take;
    }

    lines
}

fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Top-down drawing wrapper around `pdf_writer::Content` using US Letter coordinates (`y=0` at top).
struct PdfCanvas {
    content: Content,
}

impl PdfCanvas {
    fn new() -> Self {
        Self {
            content: Content::new(),
        }
    }

    fn finish(self) -> Vec<u8> {
        self.content.finish().into_vec()
    }

    #[inline]
    fn y_pdf(y_top: f32) -> f32 {
        PAGE_H - y_top
    }

    fn set_fill(&mut self, (r, g, b): (u8, u8, u8)) {
        self.content
            .set_fill_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    }

    fn set_stroke(&mut self, (r, g, b): (u8, u8, u8)) {
        self.content
            .set_stroke_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    }

    fn fill_rect(&mut self, x: f32, y_top: f32, w: f32, h: f32, color: (u8, u8, u8)) {
        self.content.save_state();
        self.set_fill(color);
        self.content.rect(x, PAGE_H - (y_top + h), w, h);
        self.content.fill_nonzero();
        self.content.restore_state();
    }

    fn stroke_rect(
        &mut self,
        x: f32,
        y_top: f32,
        w: f32,
        h: f32,
        stroke: (u8, u8, u8),
        line_width: f32,
    ) {
        self.content.save_state();
        self.set_stroke(stroke);
        self.content.set_line_width(line_width);
        self.content.rect(x, PAGE_H - (y_top + h), w, h);
        self.content.stroke();
        self.content.restore_state();
    }

    fn card(
        &mut self,
        x: f32,
        y_top: f32,
        w: f32,
        h: f32,
        fill: (u8, u8, u8),
        border: (u8, u8, u8),
    ) {
        self.fill_rect(x, y_top, w, h, fill);
        self.stroke_rect(x, y_top, w, h, border, 0.75);
    }

    fn line(
        &mut self,
        x1: f32,
        y1_top: f32,
        x2: f32,
        y2_top: f32,
        color: (u8, u8, u8),
        width: f32,
        dashed: bool,
    ) {
        self.content.save_state();
        self.set_stroke(color);
        self.content.set_line_width(width);
        if dashed {
            self.content.set_dash_pattern([4.0, 3.0], 0.0);
        }
        self.content.move_to(x1, Self::y_pdf(y1_top));
        self.content.line_to(x2, Self::y_pdf(y2_top));
        self.content.stroke();
        self.content.restore_state();
    }

    fn polyline(
        &mut self,
        points: &[(f32, f32)],
        color: (u8, u8, u8),
        width: f32,
        dashed: bool,
    ) {
        if points.len() < 2 {
            return;
        }
        self.content.save_state();
        self.set_stroke(color);
        self.content.set_line_width(width);
        if dashed {
            self.content.set_dash_pattern([4.0, 2.5], 0.0);
        }
        self.content
            .move_to(points[0].0, Self::y_pdf(points[0].1));
        for &(px, py_top) in &points[1..] {
            self.content.line_to(px, Self::y_pdf(py_top));
        }
        self.content.stroke();
        self.content.restore_state();
    }

    fn text(
        &mut self,
        font: Name<'static>,
        size: f32,
        x: f32,
        baseline_top: f32,
        color: (u8, u8, u8),
        raw: &str,
    ) {
        let bytes = pdf_sanitize_winansi(raw);
        self.content.save_state();
        self.set_fill(color);
        self.content.begin_text();
        self.content.set_font(font, size);
        self.content.next_line(x, Self::y_pdf(baseline_top));
        self.content.show(Str(&bytes));
        self.content.end_text();
        self.content.restore_state();
    }

    fn estimate_width(font: Name<'static>, size: f32, bytes: &[u8]) -> f32 {
        if font == FONT_MONO {
            return bytes.len() as f32 * size * 0.6;
        }
        let bold_scale = if font == FONT_BOLD { 1.05 } else { 1.0 };
        let units: f32 = bytes
            .iter()
            .map(|&b| match b {
                b'i' | b'l' | b'I' | b'.' | b',' | b':' | b';' | b' ' | b'|' => 0.28,
                b'm' | b'M' | b'w' | b'W' => 0.78,
                b'0'..=b'9' => 0.56,
                b'A'..=b'Z' => 0.66,
                _ => 0.52,
            })
            .sum();
        units * size * bold_scale
    }

    fn text_right(
        &mut self,
        font: Name<'static>,
        size: f32,
        right_x: f32,
        baseline_top: f32,
        color: (u8, u8, u8),
        raw: &str,
    ) {
        let bytes = pdf_sanitize_winansi(raw);
        let width = Self::estimate_width(font, size, &bytes);
        let x = (right_x - width).max(MARGIN);
        self.content.save_state();
        self.set_fill(color);
        self.content.begin_text();
        self.content.set_font(font, size);
        self.content.next_line(x, Self::y_pdf(baseline_top));
        self.content.show(Str(&bytes));
        self.content.end_text();
        self.content.restore_state();
    }

    fn footer(&mut self, app_version: &str, page_num: usize, total_pages: usize) {
        self.line(MARGIN, 738.0, MARGIN + CONTENT_W, 738.0, COLOR_BORDER, 0.75, false);
        self.text(
            FONT_REGULAR,
            8.0,
            MARGIN,
            752.0,
            COLOR_MUTED,
            &format!("Generated offline by rusty-loader v{app_version}"),
        );
        self.text_right(
            FONT_REGULAR,
            8.0,
            MARGIN + CONTENT_W,
            752.0,
            COLOR_MUTED,
            &format!("Page {page_num} of {total_pages}"),
        );
    }
}

struct ChartSeries<'a> {
    label: &'a str,
    color: (u8, u8, u8),
    dashed: bool,
    right_axis: bool,
    values: Vec<f64>,
}

struct VectorChartSpec<'a> {
    title: &'a str,
    x: f32,
    y_top: f32,
    w: f32,
    h: f32,
    left_label: &'a str,
    right_label: Option<&'a str>,
    t_min: f64,
    t_max: f64,
    series: Vec<ChartSeries<'a>>,
}

fn nice_ceiling(raw_max: f64) -> f64 {
    if !raw_max.is_finite() || raw_max <= 0.0 {
        return 1.0;
    }
    let exp = raw_max.log10().floor();
    let base = 10_f64.powf(exp);
    let frac = raw_max / base;
    let step = if frac <= 1.0 {
        1.0
    } else if frac <= 1.5 {
        1.5
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 3.0 {
        3.0
    } else if frac <= 5.0 {
        5.0
    } else if frac <= 7.5 {
        7.5
    } else {
        10.0
    };
    (step * base).max(1e-6)
}

fn format_axis_val(val: f64) -> String {
    if val >= 1000.0 {
        format!("{val:.0}")
    } else if val >= 10.0 {
        format!("{val:.1}")
    } else if val >= 1.0 {
        format!("{val:.2}")
    } else {
        format!("{val:.3}")
    }
}

fn draw_vector_chart(canvas: &mut PdfCanvas, spec: &VectorChartSpec<'_>) {
    canvas.card(spec.x, spec.y_top, spec.w, spec.h, COLOR_WHITE, COLOR_BORDER);

    // Header band inside card
    canvas.fill_rect(spec.x + 0.5, spec.y_top + 0.5, spec.w - 1.0, 22.0, COLOR_CARD_BG);
    canvas.line(
        spec.x,
        spec.y_top + 22.5,
        spec.x + spec.w,
        spec.y_top + 22.5,
        COLOR_BORDER,
        0.5,
        false,
    );
    canvas.text(
        FONT_BOLD,
        9.5,
        spec.x + 10.0,
        spec.y_top + 14.5,
        COLOR_INK,
        spec.title,
    );

    // Inline legend right-aligned on header band
    let mut legend_x = spec.x + spec.w - 10.0;
    for s in spec.series.iter().rev() {
        let label_w = PdfCanvas::estimate_width(FONT_REGULAR, 7.5, s.label.as_bytes());
        legend_x -= label_w;
        canvas.text(
            FONT_REGULAR,
            7.5,
            legend_x,
            spec.y_top + 14.0,
            COLOR_SECONDARY,
            s.label,
        );
        legend_x -= 14.0;
        canvas.line(
            legend_x,
            spec.y_top + 11.5,
            legend_x + 10.0,
            spec.y_top + 11.5,
            s.color,
            1.6,
            s.dashed,
        );
        legend_x -= 10.0;
    }

    let left_pad = 50.0;
    let right_pad = if spec.right_label.is_some() { 46.0 } else { 18.0 };
    let top_pad = 34.0;
    let bottom_pad = 24.0;

    let plot_x = spec.x + left_pad;
    let plot_y = spec.y_top + top_pad;
    let plot_w = (spec.w - left_pad - right_pad).max(10.0);
    let plot_h = (spec.h - top_pad - bottom_pad).max(10.0);

    let left_raw_max = spec
        .series
        .iter()
        .filter(|s| !s.right_axis)
        .flat_map(|s| s.values.iter().copied())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .fold(0.0_f64, f64::max);
    let left_max = nice_ceiling(left_raw_max);

    let right_raw_max = spec
        .series
        .iter()
        .filter(|s| s.right_axis)
        .flat_map(|s| s.values.iter().copied())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .fold(0.0_f64, f64::max);
    let right_max = nice_ceiling(right_raw_max);

    // Axis unit labels
    canvas.text(
        FONT_BOLD,
        7.0,
        spec.x + 8.0,
        plot_y - 4.0,
        COLOR_MUTED,
        spec.left_label,
    );
    if let Some(r_label) = spec.right_label {
        canvas.text_right(
            FONT_BOLD,
            7.0,
            spec.x + spec.w - 8.0,
            plot_y - 4.0,
            COLOR_MUTED,
            r_label,
        );
    }

    // Horizontal grid lines (4 intervals = 5 tick marks)
    let ticks = 4usize;
    for i in 0..=ticks {
        let frac = i as f32 / ticks as f32;
        let gy = plot_y + plot_h - frac * plot_h;
        canvas.line(
            plot_x,
            gy,
            plot_x + plot_w,
            gy,
            COLOR_BORDER,
            if i == 0 { 0.8 } else { 0.45 },
            i != 0,
        );

        let left_val = left_max * (frac as f64);
        canvas.text_right(
            FONT_REGULAR,
            7.0,
            plot_x - 5.0,
            gy + 2.5,
            COLOR_MUTED,
            &format_axis_val(left_val),
        );

        if spec.right_label.is_some() {
            let right_val = right_max * (frac as f64);
            canvas.text(
                FONT_REGULAR,
                7.0,
                plot_x + plot_w + 5.0,
                gy + 2.5,
                COLOR_MUTED,
                &format_axis_val(right_val),
            );
        }
    }

    // Vertical time ticks (left, mid-left, mid-right, right)
    let t_span = (spec.t_max - spec.t_min).max(0.001);
    for i in 0..=3 {
        let frac = i as f32 / 3.0;
        let tx = plot_x + frac * plot_w;
        canvas.line(tx, plot_y, tx, plot_y + plot_h, COLOR_BORDER, 0.4, i != 0);
        let t_val = spec.t_min + t_span * (frac as f64);
        let label = format!("{t_val:.1}s");
        if i == 3 {
            canvas.text_right(
                FONT_REGULAR,
                7.0,
                plot_x + plot_w,
                plot_y + plot_h + 11.0,
                COLOR_MUTED,
                &label,
            );
        } else {
            canvas.text(
                FONT_REGULAR,
                7.0,
                tx,
                plot_y + plot_h + 11.0,
                COLOR_MUTED,
                &label,
            );
        }
    }

    // Plot box left border
    canvas.line(plot_x, plot_y, plot_x, plot_y + plot_h, COLOR_BORDER, 0.8, false);
    if spec.right_label.is_some() {
        canvas.line(
            plot_x + plot_w,
            plot_y,
            plot_x + plot_w,
            plot_y + plot_h,
            COLOR_BORDER,
            0.8,
            false,
        );
    }

    // Plot each vector series polyline
    for s in &spec.series {
        if s.values.len() < 2 {
            continue;
        }
        let y_max = if s.right_axis { right_max } else { left_max };
        let denom = (s.values.len() - 1) as f32;
        let mut pts = Vec::with_capacity(s.values.len());
        for (i, &val) in s.values.iter().enumerate() {
            let safe_val = if val.is_finite() { val.max(0.0) } else { 0.0 };
            let norm_y = (safe_val / y_max).clamp(0.0, 1.0) as f32;
            let px = plot_x + (i as f32 / denom) * plot_w;
            let py_top = plot_y + plot_h - norm_y * plot_h;
            pts.push((px, py_top));
        }
        canvas.polyline(&pts, s.color, 1.35, s.dashed);
    }
}

fn draw_page_one(canvas: &mut PdfCanvas, report: &ExportReport, total_pages: usize) {
    let stats = &report.stats;
    let total_errors = stats.client_errors + stats.server_errors + stats.transport_errors;
    let (success_pct, error_pct) = if stats.total_requests > 0 {
        (
            (stats.successful_requests as f64 / stats.total_requests as f64) * 100.0,
            (total_errors as f64 / stats.total_requests as f64) * 100.0,
        )
    } else {
        (0.0, 0.0)
    };

    // 1. Header Banner (y: 36..94)
    canvas.fill_rect(MARGIN, 36.0, CONTENT_W, 58.0, COLOR_INK);
    canvas.fill_rect(MARGIN, 36.0, 4.0, 58.0, COLOR_SKY);
    canvas.text(
        FONT_BOLD,
        14.0,
        MARGIN + 14.0,
        58.0,
        COLOR_WHITE,
        &format!("rusty-loader v{} -- Load Test Report", report.app_version),
    );
    let subtitle = match report.preset_name {
        Some(ref name) if !name.trim().is_empty() => format!(
            "Generated UTC: {}  |  Preset: {}",
            report.timestamp,
            truncate_str(name.trim(), 36)
        ),
        _ => format!("Generated UTC: {}", report.timestamp),
    };
    canvas.text(
        FONT_REGULAR,
        8.5,
        MARGIN + 14.0,
        76.0,
        COLOR_BANNER_MUTED,
        &subtitle,
    );
    canvas.text_right(
        FONT_BOLD,
        11.0,
        MARGIN + CONTENT_W - 12.0,
        58.0,
        COLOR_WHITE,
        &format!("{}  |  {} VUs", report.method, report.concurrency),
    );
    canvas.text_right(
        FONT_REGULAR,
        8.5,
        MARGIN + CONTENT_W - 12.0,
        76.0,
        COLOR_BANNER_MUTED,
        &format!("Elapsed: {:.2}s", stats.total_duration_secs),
    );

    // 2. Target Configuration & Redacted Headers Card (y: 106..248)
    canvas.card(MARGIN, 106.0, CONTENT_W, 142.0, COLOR_CARD_BG, COLOR_BORDER);
    canvas.fill_rect(MARGIN, 106.0, CONTENT_W, 20.0, COLOR_ROW_ALT);
    canvas.line(MARGIN, 126.0, MARGIN + CONTENT_W, 126.0, COLOR_BORDER, 0.6, false);
    canvas.text(
        FONT_BOLD,
        9.0,
        MARGIN + 10.0,
        119.5,
        COLOR_INK,
        "TARGET CONFIGURATION & REDACTED HEADERS",
    );

    canvas.text(FONT_BOLD, 8.0, MARGIN + 10.0, 141.0, COLOR_SECONDARY, "Target URL:");
    let url_lines = wrap_str(&sanitize_ascii_display(&report.url), 82, 2);
    for (idx, line) in url_lines.iter().enumerate() {
        canvas.text(
            FONT_MONO,
            8.0,
            MARGIN + 68.0,
            141.0 + (idx as f32 * 11.0),
            COLOR_INK,
            line,
        );
    }

    canvas.text(
        FONT_REGULAR,
        8.0,
        MARGIN + 10.0,
        169.0,
        COLOR_SECONDARY,
        &format!(
            "Method: {}    |    VUs: {}    |    Target Duration: {:.1}s    |    Ramp-Up: {:.1}s    |    Timeout: {} ms",
            report.method,
            report.concurrency,
            report.configured_duration_secs,
            report.ramp_up_secs,
            report.timeout_ms
        ),
    );
    canvas.line(
        MARGIN + 10.0,
        177.0,
        MARGIN + CONTENT_W - 10.0,
        177.0,
        COLOR_BORDER,
        0.5,
        false,
    );

    canvas.text(
        FONT_BOLD,
        8.0,
        MARGIN + 10.0,
        190.0,
        COLOR_SECONDARY,
        "Request Headers (sensitive values redacted):",
    );
    let mut header_pairs: Vec<(&String, &String)> = report.headers.iter().collect();
    header_pairs.sort_by(|a, b| a.0.cmp(b.0));

    if header_pairs.is_empty() {
        canvas.text(
            FONT_REGULAR,
            8.0,
            MARGIN + 10.0,
            205.0,
            COLOR_MUTED,
            "None configured.",
        );
    } else {
        // Render up to 6 headers cleanly across 2 columns x 3 rows; collapse overflow cleanly.
        let max_slots = 6usize;
        let overflow = header_pairs.len().saturating_sub(5);
        let display_count = if header_pairs.len() > max_slots {
            5
        } else {
            header_pairs.len()
        };

        for (i, (k, v)) in header_pairs.iter().take(display_count).enumerate() {
            let col = i % 2;
            let row = i / 2;
            let hx = MARGIN + 10.0 + (col as f32 * 265.0);
            let hy = 205.0 + (row as f32 * 13.0);
            let line = truncate_str(&format!("{k}: {v}"), 48);
            canvas.text(FONT_MONO, 7.5, hx, hy, COLOR_INK, &line);
        }
        if header_pairs.len() > max_slots {
            let col = 5 % 2;
            let row = 5 / 2;
            let hx = MARGIN + 10.0 + (col as f32 * 265.0);
            let hy = 205.0 + (row as f32 * 13.0);
            canvas.text(
                FONT_BOLD,
                7.5,
                hx,
                hy,
                COLOR_ACCENT,
                &format!("(+{} more headers)", overflow),
            );
        }
    }

    // 3. KPI Strip: 6 Cards (y: 260..378)
    let card_gap = 12.0;
    let card_w = (CONTENT_W - card_gap * 2.0) / 3.0; // 172.0 pt
    let card_h = 54.0;
    let kpis = [
        (
            "TOTAL REQUESTS",
            format_count(stats.total_requests),
            format!(
                "2xx: {}  |  Err: {}",
                format_count(stats.successful_requests),
                format_count(total_errors)
            ),
            COLOR_INK,
        ),
        (
            "AVG THROUGHPUT",
            format!("{:.1} req/s", stats.requests_per_sec),
            format!("Concurrency: {} VUs", report.concurrency),
            COLOR_ACCENT,
        ),
        (
            "DATA TRANSFERRED",
            format_bytes(stats.total_bytes),
            format!("Rate: {}", format_transfer_rate(stats.bytes_per_sec)),
            COLOR_INK,
        ),
        (
            "SUCCESS RATE (2XX)",
            format!("{success_pct:.2}%"),
            format!("Other statuses: {}", format_count(stats.other_statuses)),
            COLOR_EMERALD,
        ),
        (
            "ERROR RATE",
            format!("{error_pct:.2}%"),
            format!(
                "4xx: {} | 5xx: {} | Net: {}",
                stats.client_errors, stats.server_errors, stats.transport_errors
            ),
            if total_errors > 0 { COLOR_RED } else { COLOR_INK },
        ),
        (
            "MEDIAN / P99 LATENCY",
            format!("{:.2} / {:.2} ms", stats.latencies.p50_ms, stats.latencies.p99_ms),
            format!(
                "Mean: {:.2} ms  |  Max: {:.2} ms",
                stats.latencies.mean_ms, stats.latencies.max_ms
            ),
            COLOR_INK,
        ),
    ];

    for (idx, (label, value, sub, val_color)) in kpis.iter().enumerate() {
        let col = (idx % 3) as f32;
        let row = (idx / 3) as f32;
        let cx = MARGIN + col * (card_w + card_gap);
        let cy = 260.0 + row * (card_h + 10.0);
        canvas.card(cx, cy, card_w, card_h, COLOR_WHITE, COLOR_BORDER);
        canvas.fill_rect(cx, cy, card_w, 3.0, COLOR_ACCENT_SOFT);
        canvas.text(FONT_BOLD, 7.0, cx + 9.0, cy + 15.0, COLOR_MUTED, label);
        canvas.text(FONT_BOLD, 13.0, cx + 9.0, cy + 33.5, *val_color, value);
        canvas.text(FONT_REGULAR, 7.2, cx + 9.0, cy + 46.5, COLOR_SECONDARY, sub);
    }

    // 4. Cumulative HDR Latency Percentiles Table (y: 390..454)
    canvas.text(
        FONT_BOLD,
        9.5,
        MARGIN,
        396.0,
        COLOR_INK,
        "Cumulative Latency Percentiles (HDR Histogram)",
    );
    let lat = &stats.latencies;
    let lat_cols: [(&str, String); 9] = [
        ("Min", format_latency(lat.min_ms)),
        ("Mean", format_latency(lat.mean_ms)),
        ("p50", format_latency(lat.p50_ms)),
        ("p90", format_latency(lat.p90_ms)),
        ("p95", format_latency(lat.p95_ms)),
        ("p99", format_latency(lat.p99_ms)),
        ("p99.9", format_latency(lat.p99_9_ms)),
        ("Max", format_latency(lat.max_ms)),
        ("Std Dev", format_latency(lat.std_dev_ms)),
    ];
    let table_top = 404.0;
    let col_w = CONTENT_W / 9.0; // 60.0 pt
    canvas.card(MARGIN, table_top, CONTENT_W, 44.0, COLOR_WHITE, COLOR_BORDER);
    canvas.fill_rect(MARGIN + 0.5, table_top + 0.5, CONTENT_W - 1.0, 20.0, COLOR_ROW_ALT);
    canvas.line(
        MARGIN,
        table_top + 20.5,
        MARGIN + CONTENT_W,
        table_top + 20.5,
        COLOR_BORDER,
        0.6,
        false,
    );
    for (i, (hdr, val)) in lat_cols.iter().enumerate() {
        let cell_x = MARGIN + i as f32 * col_w;
        if i > 0 {
            canvas.line(
                cell_x,
                table_top,
                cell_x,
                table_top + 44.0,
                COLOR_BORDER,
                0.45,
                false,
            );
        }
        canvas.text_right(
            FONT_BOLD,
            7.5,
            cell_x + col_w - 6.0,
            table_top + 13.5,
            COLOR_SECONDARY,
            hdr,
        );
        canvas.text_right(
            FONT_REGULAR,
            8.0,
            cell_x + col_w - 6.0,
            table_top + 35.0,
            COLOR_INK,
            val,
        );
    }

    // 5. Status Code & Transport Error Breakdown Tables (y: 468..720)
    let split_gap = 16.0;
    let half_w = (CONTENT_W - split_gap) / 2.0; // 262.0 pt
    draw_status_table(canvas, MARGIN, 468.0, half_w, stats.total_requests, &stats.status_distribution);
    draw_error_table(
        canvas,
        MARGIN + half_w + split_gap,
        468.0,
        half_w,
        &stats.error_breakdown,
    );

    canvas.footer(&report.app_version, 1, total_pages);
}

fn draw_status_table(
    canvas: &mut PdfCanvas,
    x: f32,
    y_top: f32,
    w: f32,
    total_requests: u64,
    dist: &HashMap<u16, u64>,
) {
    canvas.text(
        FONT_BOLD,
        9.5,
        x,
        y_top + 8.0,
        COLOR_INK,
        "HTTP Status Distribution",
    );
    let box_y = y_top + 16.0;
    let row_h = 19.0;
    let hdr_h = 20.0;
    let max_rows = 10usize;

    let mut rows: Vec<(u16, u64)> = dist.iter().map(|(&c, &n)| (c, n)).collect();
    rows.sort_by_key(|&(code, _)| code);

    let overflow = rows.len().saturating_sub(max_rows - 1);
    let clamped = rows.len() > max_rows;
    let visible_count = if rows.is_empty() {
        1
    } else if clamped {
        max_rows
    } else {
        rows.len()
    };
    let box_h = hdr_h + (visible_count as f32 * row_h) + 2.0;

    canvas.card(x, box_y, w, box_h, COLOR_WHITE, COLOR_BORDER);
    canvas.fill_rect(x + 0.5, box_y + 0.5, w - 1.0, hdr_h, COLOR_ROW_ALT);
    canvas.line(x, box_y + hdr_h, x + w, box_y + hdr_h, COLOR_BORDER, 0.6, false);

    canvas.text(FONT_BOLD, 7.5, x + 8.0, box_y + 13.5, COLOR_SECONDARY, "Code");
    canvas.text(FONT_BOLD, 7.5, x + 46.0, box_y + 13.5, COLOR_SECONDARY, "Reason");
    canvas.text_right(FONT_BOLD, 7.5, x + w - 58.0, box_y + 13.5, COLOR_SECONDARY, "Count");
    canvas.text_right(FONT_BOLD, 7.5, x + w - 8.0, box_y + 13.5, COLOR_SECONDARY, "Share");

    if rows.is_empty() {
        canvas.text(
            FONT_REGULAR,
            8.0,
            x + 8.0,
            box_y + hdr_h + 13.0,
            COLOR_MUTED,
            "No HTTP status codes recorded.",
        );
        return;
    }

    let item_limit = if clamped { max_rows - 1 } else { rows.len() };
    for (idx, &(code, count)) in rows.iter().take(item_limit).enumerate() {
        let ry = box_y + hdr_h + (idx as f32 * row_h);
        if idx % 2 == 1 {
            canvas.fill_rect(x + 0.5, ry, w - 1.0, row_h, COLOR_CARD_BG);
        }
        let reason = StatusCode::from_u16(code)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("Unknown");
        let share = if total_requests > 0 {
            (count as f64 / total_requests as f64) * 100.0
        } else {
            0.0
        };
        let code_color = if (200..300).contains(&code) {
            COLOR_EMERALD
        } else if (400..500).contains(&code) {
            COLOR_AMBER
        } else if code >= 500 {
            COLOR_RED
        } else {
            COLOR_INK
        };
        let text_y = ry + 13.0;
        canvas.text(FONT_BOLD, 8.0, x + 8.0, text_y, code_color, &code.to_string());
        canvas.text(
            FONT_REGULAR,
            8.0,
            x + 46.0,
            text_y,
            COLOR_INK,
            &truncate_str(reason, 18),
        );
        canvas.text_right(
            FONT_REGULAR,
            8.0,
            x + w - 58.0,
            text_y,
            COLOR_INK,
            &format_count(count),
        );
        canvas.text_right(
            FONT_REGULAR,
            8.0,
            x + w - 8.0,
            text_y,
            COLOR_SECONDARY,
            &format!("{share:.1}%"),
        );
    }

    if clamped {
        let ry = box_y + hdr_h + ((max_rows - 1) as f32 * row_h);
        let extra_count: u64 = rows.iter().skip(max_rows - 1).map(|&(_, c)| c).sum();
        let text_y = ry + 13.0;
        canvas.text(
            FONT_BOLD,
            7.8,
            x + 8.0,
            text_y,
            COLOR_ACCENT,
            &format!("(+{} more status codes)", overflow),
        );
        canvas.text_right(
            FONT_REGULAR,
            8.0,
            x + w - 58.0,
            text_y,
            COLOR_MUTED,
            &format_count(extra_count),
        );
    }
}

fn draw_error_table(
    canvas: &mut PdfCanvas,
    x: f32,
    y_top: f32,
    w: f32,
    breakdown: &HashMap<String, u64>,
) {
    canvas.text(
        FONT_BOLD,
        9.5,
        x,
        y_top + 8.0,
        COLOR_INK,
        "Transport Error Breakdown",
    );
    let box_y = y_top + 16.0;
    let row_h = 19.0;
    let hdr_h = 20.0;
    let max_rows = 10usize;

    let mut rows: Vec<(&String, u64)> = breakdown.iter().map(|(k, &v)| (k, v)).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let overflow = rows.len().saturating_sub(max_rows - 1);
    let clamped = rows.len() > max_rows;
    let visible_count = if rows.is_empty() {
        1
    } else if clamped {
        max_rows
    } else {
        rows.len()
    };
    let box_h = hdr_h + (visible_count as f32 * row_h) + 2.0;

    canvas.card(x, box_y, w, box_h, COLOR_WHITE, COLOR_BORDER);
    canvas.fill_rect(x + 0.5, box_y + 0.5, w - 1.0, hdr_h, COLOR_ROW_ALT);
    canvas.line(x, box_y + hdr_h, x + w, box_y + hdr_h, COLOR_BORDER, 0.6, false);

    canvas.text(FONT_BOLD, 7.5, x + 8.0, box_y + 13.5, COLOR_SECONDARY, "Error Category");
    canvas.text_right(FONT_BOLD, 7.5, x + w - 8.0, box_y + 13.5, COLOR_SECONDARY, "Count");

    if rows.is_empty() {
        canvas.text(
            FONT_REGULAR,
            8.0,
            x + 8.0,
            box_y + hdr_h + 13.0,
            COLOR_EMERALD,
            "None (0 transport errors)",
        );
        canvas.text_right(
            FONT_REGULAR,
            8.0,
            x + w - 8.0,
            box_y + hdr_h + 13.0,
            COLOR_MUTED,
            "0",
        );
        return;
    }

    let item_limit = if clamped { max_rows - 1 } else { rows.len() };
    for (idx, (category, count)) in rows.iter().take(item_limit).enumerate() {
        let ry = box_y + hdr_h + (idx as f32 * row_h);
        if idx % 2 == 1 {
            canvas.fill_rect(x + 0.5, ry, w - 1.0, row_h, COLOR_CARD_BG);
        }
        let text_y = ry + 13.0;
        canvas.text(
            FONT_MONO,
            7.8,
            x + 8.0,
            text_y,
            COLOR_INK,
            &truncate_str(&sanitize_ascii_display(category), 32),
        );
        canvas.text_right(
            FONT_BOLD,
            8.0,
            x + w - 8.0,
            text_y,
            COLOR_RED,
            &format_count(*count),
        );
    }

    if clamped {
        let ry = box_y + hdr_h + ((max_rows - 1) as f32 * row_h);
        let extra_count: u64 = rows.iter().skip(max_rows - 1).map(|&(_, c)| c).sum();
        let text_y = ry + 13.0;
        canvas.text(
            FONT_BOLD,
            7.8,
            x + 8.0,
            text_y,
            COLOR_ACCENT,
            &format!("(+{} more categories)", overflow),
        );
        canvas.text_right(
            FONT_REGULAR,
            8.0,
            x + w - 8.0,
            text_y,
            COLOR_MUTED,
            &format_count(extra_count),
        );
    }
}

fn compute_per_tick_deltas(snapshots: &[TelemetrySnapshot]) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut ok2xx = Vec::with_capacity(snapshots.len());
    let mut err4xx = Vec::with_capacity(snapshots.len());
    let mut err5xx = Vec::with_capacity(snapshots.len());
    let mut err_net = Vec::with_capacity(snapshots.len());

    for (i, cur) in snapshots.iter().enumerate() {
        let prev = if i > 0 { Some(&snapshots[i - 1]) } else { None };
        ok2xx.push(cur.successful_requests.saturating_sub(prev.map_or(0, |p| p.successful_requests)) as f64);
        err4xx.push(cur.client_errors.saturating_sub(prev.map_or(0, |p| p.client_errors)) as f64);
        err5xx.push(cur.server_errors.saturating_sub(prev.map_or(0, |p| p.server_errors)) as f64);
        err_net.push(cur.transport_errors.saturating_sub(prev.map_or(0, |p| p.transport_errors)) as f64);
    }

    (ok2xx, err4xx, err5xx, err_net)
}

fn draw_page_two(
    canvas: &mut PdfCanvas,
    report: &ExportReport,
    page_num: usize,
    total_pages: usize,
) {
    let snaps = &report.snapshots;
    let t_min = snaps.first().map_or(0.0, |s| s.elapsed_secs);
    let t_max = snaps.last().map_or(report.stats.total_duration_secs, |s| s.elapsed_secs);

    // Header bar
    canvas.card(MARGIN, 36.0, CONTENT_W, 34.0, COLOR_CARD_BG, COLOR_BORDER);
    canvas.fill_rect(MARGIN, 36.0, 4.0, 34.0, COLOR_ACCENT);
    canvas.text(
        FONT_BOLD,
        11.0,
        MARGIN + 12.0,
        56.5,
        COLOR_INK,
        "Time-Series Telemetry Charts",
    );
    canvas.text_right(
        FONT_MONO,
        8.0,
        MARGIN + CONTENT_W - 10.0,
        56.0,
        COLOR_SECONDARY,
        &truncate_str(
            &format!("{} {}  |  {} ticks", report.method, sanitize_ascii_display(&report.url), snaps.len()),
            62,
        ),
    );

    // Chart 1: Throughput & Active VUs (y: 82..282)
    draw_vector_chart(
        canvas,
        &VectorChartSpec {
            title: "Throughput & Active Virtual Users",
            x: MARGIN,
            y_top: 82.0,
            w: CONTENT_W,
            h: 200.0,
            left_label: "req/s",
            right_label: Some("VUs"),
            t_min,
            t_max,
            series: vec![
                ChartSeries {
                    label: "Window RPS",
                    color: COLOR_SKY,
                    dashed: false,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.window_rps).collect(),
                },
                ChartSeries {
                    label: "Avg RPS",
                    color: COLOR_INDIGO,
                    dashed: true,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.avg_rps).collect(),
                },
                ChartSeries {
                    label: "Active VUs",
                    color: COLOR_AMBER,
                    dashed: false,
                    right_axis: true,
                    values: snaps.iter().map(|s| s.active_workers as f64).collect(),
                },
            ],
        },
    );

    // Chart 2: Windowed HDR Latency Percentiles (y: 298..498)
    draw_vector_chart(
        canvas,
        &VectorChartSpec {
            title: "Windowed HDR Latency Percentiles",
            x: MARGIN,
            y_top: 298.0,
            w: CONTENT_W,
            h: 200.0,
            left_label: "ms",
            right_label: None,
            t_min,
            t_max,
            series: vec![
                ChartSeries {
                    label: "p50",
                    color: COLOR_EMERALD,
                    dashed: false,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.window_latency_ms.p50).collect(),
                },
                ChartSeries {
                    label: "p90",
                    color: COLOR_SKY_LIGHT,
                    dashed: false,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.window_latency_ms.p90).collect(),
                },
                ChartSeries {
                    label: "p95",
                    color: COLOR_AMBER,
                    dashed: false,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.window_latency_ms.p95).collect(),
                },
                ChartSeries {
                    label: "p99",
                    color: COLOR_ROSE,
                    dashed: false,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.window_latency_ms.p99).collect(),
                },
                ChartSeries {
                    label: "max",
                    color: COLOR_PURPLE,
                    dashed: true,
                    right_axis: false,
                    values: snaps.iter().map(|s| s.window_latency_ms.max).collect(),
                },
            ],
        },
    );

    // Chart 3: Per-Tick Responses & Errors by Class (y: 514..714)
    let (ok2xx, err4xx, err5xx, err_net) = compute_per_tick_deltas(snaps);
    draw_vector_chart(
        canvas,
        &VectorChartSpec {
            title: "Per-Tick Responses & Errors by Class",
            x: MARGIN,
            y_top: 514.0,
            w: CONTENT_W,
            h: 200.0,
            left_label: "count",
            right_label: None,
            t_min,
            t_max,
            series: vec![
                ChartSeries {
                    label: "2xx",
                    color: COLOR_EMERALD,
                    dashed: false,
                    right_axis: false,
                    values: ok2xx,
                },
                ChartSeries {
                    label: "4xx",
                    color: COLOR_AMBER,
                    dashed: false,
                    right_axis: false,
                    values: err4xx,
                },
                ChartSeries {
                    label: "5xx",
                    color: COLOR_RED,
                    dashed: false,
                    right_axis: false,
                    values: err5xx,
                },
                ChartSeries {
                    label: "Transport",
                    color: COLOR_PINK,
                    dashed: false,
                    right_axis: false,
                    values: err_net,
                },
            ],
        },
    );

    canvas.footer(&report.app_version, page_num, total_pages);
}

fn draw_scenario_page(
    canvas: &mut PdfCanvas,
    report: &ExportReport,
    sc: &crate::metrics::stats::ScenarioStats,
    page_num: usize,
    total_pages: usize,
) {
    canvas.card(MARGIN, 36.0, CONTENT_W, 34.0, COLOR_CARD_BG, COLOR_BORDER);
    canvas.fill_rect(MARGIN, 36.0, 4.0, 34.0, COLOR_ACCENT);
    canvas.text(
        FONT_BOLD,
        11.0,
        MARGIN + 12.0,
        56.5,
        COLOR_INK,
        "Ordered API Sequence: Journey & Per-Step Breakdown",
    );
    let seq_title = report
        .scenario
        .as_ref()
        .and_then(|s| s.name.as_deref())
        .unwrap_or("Sequence Run");
    canvas.text_right(
        FONT_MONO,
        8.0,
        MARGIN + CONTENT_W - 10.0,
        56.0,
        COLOR_SECONDARY,
        &truncate_str(
            &format!("{}  |  {} steps", sanitize_ascii_display(seq_title), sc.steps.len()),
            48,
        ),
    );

    // Journey summary card (y: 84..148)
    let j = &sc.journeys;
    canvas.card(MARGIN, 84.0, CONTENT_W, 64.0, COLOR_WHITE, COLOR_BORDER);
    canvas.fill_rect(MARGIN + 0.5, 84.5, CONTENT_W - 1.0, 20.0, COLOR_ROW_ALT);
    canvas.text(
        FONT_BOLD,
        8.5,
        MARGIN + 10.0,
        98.0,
        COLOR_INK,
        "End-to-End Journey Summary",
    );

    let kpis = [
        ("Started", format_count(j.started_iterations)),
        (
            "Completed",
            format!("{} ({:.1}%)", format_count(j.completed_iterations), j.completion_rate_pct),
        ),
        ("Aborted", format_count(j.aborted_iterations)),
        ("Iter / sec", format!("{:.2}", j.iterations_per_sec)),
        ("p50 / p95 / p99", format!("{:.1} / {:.1} / {:.1} ms", j.duration.p50_ms, j.duration.p95_ms, j.duration.p99_ms)),
    ];
    let col_w = CONTENT_W / kpis.len() as f32;
    for (idx, (label, val)) in kpis.iter().enumerate() {
        let kx = MARGIN + 10.0 + (idx as f32 * col_w);
        canvas.text(FONT_REGULAR, 7.5, kx, 120.0, COLOR_MUTED, label);
        canvas.text(FONT_BOLD, 8.8, kx, 136.0, COLOR_INK, val);
    }

    // Per-Step Table (y: 164..)
    canvas.text(
        FONT_BOLD,
        9.5,
        MARGIN,
        172.0,
        COLOR_INK,
        "Per-Step Performance & Reliability",
    );
    let box_y = 180.0;
    let hdr_h = 22.0;
    let row_h = 21.0;
    let max_rows = 24usize;
    let visible = sc.steps.len().min(max_rows);
    let box_h = hdr_h + (visible.max(1) as f32 * row_h) + 4.0;

    canvas.card(MARGIN, box_y, CONTENT_W, box_h, COLOR_WHITE, COLOR_BORDER);
    canvas.fill_rect(MARGIN + 0.5, box_y + 0.5, CONTENT_W - 1.0, hdr_h, COLOR_ROW_ALT);
    canvas.line(MARGIN, box_y + hdr_h, MARGIN + CONTENT_W, box_y + hdr_h, COLOR_BORDER, 0.6, false);

    let hy = box_y + 14.5;
    canvas.text(FONT_BOLD, 7.5, MARGIN + 8.0, hy, COLOR_SECONDARY, "#");
    canvas.text(FONT_BOLD, 7.5, MARGIN + 26.0, hy, COLOR_SECONDARY, "Step");
    canvas.text(FONT_BOLD, 7.5, MARGIN + 150.0, hy, COLOR_SECONDARY, "Method");
    canvas.text_right(FONT_BOLD, 7.5, MARGIN + 250.0, hy, COLOR_SECONDARY, "Reqs");
    canvas.text_right(FONT_BOLD, 7.5, MARGIN + 305.0, hy, COLOR_SECONDARY, "RPS");
    canvas.text_right(FONT_BOLD, 7.5, MARGIN + 365.0, hy, COLOR_SECONDARY, "p50 ms");
    canvas.text_right(FONT_BOLD, 7.5, MARGIN + 425.0, hy, COLOR_SECONDARY, "p95 ms");
    canvas.text_right(FONT_BOLD, 7.5, MARGIN + 480.0, hy, COLOR_SECONDARY, "Errors");
    canvas.text_right(FONT_BOLD, 7.5, MARGIN + CONTENT_W - 8.0, hy, COLOR_SECONDARY, "Aborts");

    for (idx, step) in sc.steps.iter().take(max_rows).enumerate() {
        let ry = box_y + hdr_h + (idx as f32 * row_h);
        if idx % 2 == 1 {
            canvas.fill_rect(MARGIN + 0.5, ry, CONTENT_W - 1.0, row_h, COLOR_CARD_BG);
        }
        let ty = ry + 14.0;
        let errs = step
            .total_requests
            .saturating_sub(step.successful_requests + step.other_statuses);
        canvas.text(FONT_BOLD, 7.8, MARGIN + 8.0, ty, COLOR_SECONDARY, &step.order_number.to_string());
        canvas.text(
            FONT_REGULAR,
            7.8,
            MARGIN + 26.0,
            ty,
            COLOR_INK,
            &truncate_str(&sanitize_ascii_display(&step.step_name), 22),
        );
        canvas.text(FONT_MONO, 7.8, MARGIN + 150.0, ty, COLOR_ACCENT, &step.method);
        canvas.text_right(FONT_REGULAR, 7.8, MARGIN + 250.0, ty, COLOR_INK, &format_count(step.total_requests));
        canvas.text_right(FONT_REGULAR, 7.8, MARGIN + 305.0, ty, COLOR_INK, &format!("{:.1}", step.requests_per_sec));
        canvas.text_right(FONT_REGULAR, 7.8, MARGIN + 365.0, ty, COLOR_INK, &format!("{:.1}", step.latencies.p50_ms));
        canvas.text_right(FONT_REGULAR, 7.8, MARGIN + 425.0, ty, COLOR_INK, &format!("{:.1}", step.latencies.p95_ms));
        canvas.text_right(
            FONT_REGULAR,
            7.8,
            MARGIN + 480.0,
            ty,
            if errs > 0 { COLOR_RED } else { COLOR_MUTED },
            &format_count(errs),
        );
        canvas.text_right(
            FONT_REGULAR,
            7.8,
            MARGIN + CONTENT_W - 8.0,
            ty,
            if step.aborted_iterations > 0 { COLOR_AMBER } else { COLOR_MUTED },
            &format_count(step.aborted_iterations),
        );
    }

    canvas.footer(&report.app_version, page_num, total_pages);
}

/// Generates an offline vector PDF 1.7 document from a redacted [`ExportReport`].
pub fn generate_pdf_report(report: &ExportReport) -> Result<Vec<u8>> {
    let has_charts_page = report.snapshots.len() >= 2;
    let has_scenario_page = report.stats.scenario_stats.is_some();
    let total_pages = 1 + usize::from(has_charts_page) + usize::from(has_scenario_page);

    let mut next_id = 1i32;
    let mut alloc = || {
        let r = Ref::new(next_id);
        next_id += 1;
        r
    };

    let catalog_id = alloc();
    let pages_id = alloc();
    let font_reg_id = alloc();
    let font_bold_id = alloc();
    let font_mono_id = alloc();
    let page1_id = alloc();
    let content1_id = alloc();
    let page2_pair = has_charts_page.then(|| (alloc(), alloc()));
    let page3_pair = has_scenario_page.then(|| (alloc(), alloc()));

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(pages_id);

    let mut kids = vec![page1_id];
    if let Some((p2, _)) = page2_pair {
        kids.push(p2);
    }
    if let Some((p3, _)) = page3_pair {
        kids.push(p3);
    }
    pdf.pages(pages_id).kids(kids).count(total_pages as i32);

    // Standard 14 Type1 Fonts with WinAnsi encoding
    for (font_id, base_name) in [
        (font_reg_id, Name(b"Helvetica")),
        (font_bold_id, Name(b"Helvetica-Bold")),
        (font_mono_id, Name(b"Courier")),
    ] {
        let mut f = pdf.type1_font(font_id);
        f.base_font(base_name);
        f.encoding_predefined(Name(b"WinAnsiEncoding"));
        f.finish();
    }

    // Page 1: Executive Summary & Breakdowns
    let mut canvas1 = PdfCanvas::new();
    draw_page_one(&mut canvas1, report, total_pages);
    let stream1 = canvas1.finish();
    write_pdf_page(
        &mut pdf,
        page1_id,
        pages_id,
        content1_id,
        &stream1,
        font_reg_id,
        font_bold_id,
        font_mono_id,
    );

    let mut cur_page = 2usize;

    // Optional Page 2: Vector Telemetry Charts
    if let Some((page2_id, content2_id)) = page2_pair {
        let mut canvas2 = PdfCanvas::new();
        draw_page_two(&mut canvas2, report, cur_page, total_pages);
        let stream2 = canvas2.finish();
        write_pdf_page(
            &mut pdf,
            page2_id,
            pages_id,
            content2_id,
            &stream2,
            font_reg_id,
            font_bold_id,
            font_mono_id,
        );
        cur_page += 1;
    }

    // Optional Scenario Page: Journey & Per-Step Breakdown
    if let (Some((page3_id, content3_id)), Some(ref sc)) =
        (page3_pair, &report.stats.scenario_stats)
    {
        let mut canvas3 = PdfCanvas::new();
        draw_scenario_page(&mut canvas3, report, sc, cur_page, total_pages);
        let stream3 = canvas3.finish();
        write_pdf_page(
            &mut pdf,
            page3_id,
            pages_id,
            content3_id,
            &stream3,
            font_reg_id,
            font_bold_id,
            font_mono_id,
        );
    }

    Ok(pdf.finish())
}

#[allow(clippy::too_many_arguments)]
fn write_pdf_page(
    pdf: &mut Pdf,
    page_id: Ref,
    pages_id: Ref,
    content_id: Ref,
    stream_bytes: &[u8],
    font_reg_id: Ref,
    font_bold_id: Ref,
    font_mono_id: Ref,
) {
    {
        let mut page = pdf.page(page_id);
        page.parent(pages_id);
        page.media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H));
        page.contents(content_id);
        let mut resources = page.resources();
        let mut fonts = resources.fonts();
        fonts.pair(FONT_REGULAR, font_reg_id);
        fonts.pair(FONT_BOLD, font_bold_id);
        fonts.pair(FONT_MONO, font_mono_id);
        fonts.finish();
        resources.finish();
        page.finish();
    }
    pdf.stream(content_id, stream_bytes);
}

/// Generates and writes a PDF report to `path`, creating parent directories automatically.
pub fn export_pdf_report(path: &Path, report: &ExportReport) -> Result<()> {
    let bytes = generate_pdf_report(report)?;
    write_export_bytes(path, &bytes)
}
