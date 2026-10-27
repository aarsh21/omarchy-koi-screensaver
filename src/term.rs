//! ANSI output: paints a Frame into a real terminal, sending only the cells that changed.
//!
//! Every koi cell carries a background colour (the skin tile) and a foreground colour (the etched
//! glyph). Consecutive changed cells are written as one run, so a moving body costs one cursor jump
//! and a handful of SGR sequences rather than a full-screen repaint.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::render::{pack, Frame, DARK};

pub const DEFAULT_BG: u32 = pack(DARK.bg);

const ENTER_ALT_SCREEN: &str = "\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l";
const LEAVE_ALT_SCREEN: &str = "\x1b[0m\x1b[49m\x1b[?25h\x1b[?1049l";

pub fn set_background_sequence() -> String {
    let b = DARK.bg;
    format!("\x1b]11;rgb:{:02x}/{:02x}/{:02x}\x07", b[0], b[1], b[2])
}

pub struct Terminal {
    pub cols: usize,
    pub rows: usize,
    prev: Option<Frame>,
    started: bool,
    pub bytes_written: u64,
    pub cells_painted: u64,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            prev: None,
            started: false,
            bytes_written: 0,
            cells_painted: 0,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.prev = None;
    }

    pub fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let mut out = io::stdout().lock();
        let _ = out.write_all(ENTER_ALT_SCREEN.as_bytes());
        let seq = set_background_sequence();
        self.bytes_written += (ENTER_ALT_SCREEN.len() + seq.len()) as u64;
        let _ = out.write_all(seq.as_bytes());
        let _ = out.flush();
    }

    pub fn stop(&mut self) {
        if !self.started {
            return;
        }
        self.started = false;
        let mut out = io::stdout().lock();
        let _ = out.write_all(LEAVE_ALT_SCREEN.as_bytes());
        let _ = out.flush();
    }

    /// Paint `frame`, repainting only cells whose glyph or colours changed since the last call.
    pub fn paint(&mut self, frame: &Frame) -> usize {
        let (out, painted) = self.diff(frame);
        if out.is_empty() {
            return 0;
        }
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
        self.bytes_written += out.len() as u64;
        painted
    }

    /// The ANSI payload for `frame`, given what was painted last time.
    pub fn diff(&mut self, frame: &Frame) -> (String, usize) {
        if frame.cols != self.cols || frame.rows != self.rows {
            self.resize(frame.cols, frame.rows);
        }
        let full = self.prev.is_none();
        let mut out = String::with_capacity(4096);
        if full {
            out.push_str("\x1b[2J\x1b[H");
        }
        let mut last_fg = None;
        let mut last_bg = None;
        let mut painted = 0usize;

        for r in 0..frame.rows {
            let row = r * frame.cols;
            let mut c = 0;
            while c < frame.cols {
                let i = row + c;
                if !full && self.same_cell(frame, i) {
                    c += 1;
                    continue;
                }
                // Extend the run across every changed neighbour so we move the cursor once.
                let mut end = c;
                while end < frame.cols {
                    let j = row + end;
                    if !full && self.same_cell(frame, j) {
                        break;
                    }
                    end += 1;
                }
                let _ = write!(out, "\x1b[{};{}H", r + 1, c + 1);
                for k in c..end {
                    let j = row + k;
                    let bg = frame.bg[j];
                    if Some(bg) != last_bg {
                        if bg == DEFAULT_BG {
                            out.push_str("\x1b[49m");
                        } else {
                            let _ = write!(
                                out,
                                "\x1b[48;2;{};{};{}m",
                                bg >> 16,
                                (bg >> 8) & 255,
                                bg & 255
                            );
                        }
                        last_bg = Some(bg);
                    }
                    let glyph = frame.glyph[j];
                    if glyph != b' ' {
                        let fg = frame.fg[j];
                        if Some(fg) != last_fg {
                            let _ = write!(
                                out,
                                "\x1b[38;2;{};{};{}m",
                                fg >> 16,
                                (fg >> 8) & 255,
                                fg & 255
                            );
                            last_fg = Some(fg);
                        }
                    }
                    out.push(glyph as char);
                    painted += 1;
                }
                c = end;
            }
        }

        match &mut self.prev {
            Some(prev) => {
                prev.glyph.copy_from_slice(&frame.glyph);
                prev.fg.copy_from_slice(&frame.fg);
                prev.bg.copy_from_slice(&frame.bg);
            }
            slot => *slot = Some(frame.clone()),
        }

        self.cells_painted += painted as u64;
        (out, painted)
    }

    fn same_cell(&self, frame: &Frame, i: usize) -> bool {
        match &self.prev {
            Some(p) => {
                p.glyph[i] == frame.glyph[i] && p.fg[i] == frame.fg[i] && p.bg[i] == frame.bg[i]
            }
            None => false,
        }
    }

    /// Drop the cached frame so the next paint repaints everything (used after a resize).
    pub fn invalidate(&mut self) {
        self.prev = None;
    }
}
