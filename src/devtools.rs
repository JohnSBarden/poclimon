//! Developer instrumentation, inert unless its environment variable is set.
//!
//! `POCLIMON_FRAME_DUMP=<path>`: every 10th frame, append the frame ratatui
//! *intended* to draw to `<path>` (JSON lines) and write an invisible OSC 9999
//! marker to the terminal right after that frame is flushed. The terminal
//! harness in `tools/term-harness` replays a recording up to each marker and
//! checks that the screen matches the intended frame cell for cell — the
//! regression test for the rendering bugs fixed alongside this module.

use ratatui::buffer::{Buffer, CellDiffOption};
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

fn dump_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var("POCLIMON_FRAME_DUMP")
            .ok()
            .filter(|p| !p.is_empty())
    })
    .as_deref()
}

/// Whether frame dumping is on (so callers can skip cloning the buffer).
pub fn frame_dump_enabled() -> bool {
    dump_path().is_some()
}

/// Record `buf` (see module docs). No-op when dumping is disabled.
pub fn frame_dump(buf: &Buffer) {
    static N: AtomicU64 = AtomicU64::new(0);
    let Some(path) = dump_path() else { return };
    let n = N.fetch_add(1, Ordering::Relaxed);
    if !n.is_multiple_of(10) {
        return;
    }
    let mut cells = Vec::with_capacity(buf.content.len());
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = &buf[(buf.area.x + x, buf.area.y + y)];
            let sym = c.symbol();
            let kind = if c.diff_option == CellDiffOption::Skip {
                "skip"
            } else if sym.contains('\u{10EEEE}') {
                "ph"
            } else if sym.starts_with('\u{1b}') {
                "img"
            } else {
                "txt"
            };
            let ch = if kind == "txt" {
                sym.chars().next().unwrap_or(' ').to_string()
            } else {
                String::new()
            };
            cells.push(serde_json::json!([x, y, kind, ch, format!("{:?}", c.fg)]));
        }
    }
    let line = serde_json::json!({
        "n": n, "w": buf.area.width, "h": buf.area.height, "cells": cells,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b]9999;frame={n}\x07");
    let _ = out.flush();
}
