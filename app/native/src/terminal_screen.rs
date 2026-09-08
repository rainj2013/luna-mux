//! Runtime-owned text screen. OSC/DCS strings are discarded before parsing:
//! no clipboard/title/query side effects and no unbounded control-string buffer.
use crate::terminal_runtime_contract::{
    TERMINAL_SCREEN_MAX_BYTES, TERMINAL_SCREEN_MAX_COLS, TERMINAL_SCREEN_MAX_ROWS,
    TerminalScreenModes, TerminalScreenSnapshot,
};

use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy)]
enum Filter {
    Ground,
    Escape,
    String { osc: bool },
}

pub struct TerminalScreen {
    parser: vt100::Parser,
    filter: Filter,
    size_limited: bool,
    activity_parser: vte::Parser,
    row_cursors: Vec<u64>,
    processed: u64,
    pending_c2: bool,
    diff_budget: usize,
}

impl TerminalScreen {
    pub fn new(rows: u32, cols: u32) -> Self {
        let mut screen = Self {
            parser: vt100::Parser::new(24, 80, 0),
            filter: Filter::Ground,
            size_limited: false,
            activity_parser: vte::Parser::new(),
            row_cursors: vec![0; 24],
            processed: 0,
            pending_c2: false,
            diff_budget: 0,
        };
        screen.resize(rows, cols);
        screen
    }

    pub fn resize(&mut self, rows: u32, cols: u32) {
        let r = rows.clamp(1, u32::from(TERMINAL_SCREEN_MAX_ROWS)) as u16;
        // vt100 wide-cell arithmetic requires at least two columns.
        let c = cols.clamp(2, u32::from(TERMINAL_SCREEN_MAX_COLS)) as u16;
        // Once a grid has lost cells, a later smaller resize cannot recover them.
        self.size_limited |= u32::from(r) != rows || u32::from(c) != cols;
        self.parser.screen_mut().set_size(r, c);
        self.row_cursors.resize(usize::from(r), 0);
    }

    pub fn process(&mut self, bytes: &[u8]) {
        // Bound freshness bookkeeping independently of the output chunk length.
        // vt100 continues processing; unknown row stamps are cleared, never guessed.
        self.diff_budget = 262144;
        for &byte in bytes {
            self.processed = self.processed.saturating_add(1);
            // Runtime output is UTF-8. Normalize encoded C1 controls to their
            // seven-bit equivalents before filtering, including across chunks.
            if self.pending_c2 {
                self.pending_c2 = false;
                if (0x80..=0x9f).contains(&byte) {
                    match byte {
                        0x84 => self.escape(b'D'),
                        0x85 => self.escape(b'E'),
                        0x8d => self.escape(b'M'),
                        0x90 => self.escape(b'P'),
                        0x98 => self.escape(b'X'),
                        0x9b => self.escape(b'['),
                        0x9c => self.escape(b'\\'),
                        0x9d => self.escape(b']'),
                        0x9e => self.escape(b'^'),
                        0x9f => self.escape(b'_'),
                        _ => {}
                    }
                    continue;
                }
                self.filtered(0xc2);
            }
            if byte == 0xc2 {
                self.pending_c2 = true;
            } else {
                self.filtered(byte);
            }
        }
    }

    fn escape(&mut self, byte: u8) {
        self.filtered(0x1b);
        self.filtered(byte);
    }

    fn filtered(&mut self, byte: u8) {
        self.filter = match self.filter {
            Filter::Ground if byte == 0x1b => Filter::Escape,
            Filter::Ground => {
                self.feed(byte);
                Filter::Ground
            }
            Filter::Escape => match byte {
                b']' | b'P' | b'X' | b'^' | b'_' => {
                    // Cancel any preceding incomplete CSI/UTF-8 parser state.
                    self.feed(0x18);
                    Filter::String { osc: byte == b']' }
                }
                0x1b => {
                    self.feed(0x1b);
                    Filter::Escape
                }
                _ => {
                    self.feed(0x1b);
                    self.feed(byte);
                    Filter::Ground
                }
            },
            Filter::String { osc } => {
                if byte == 0x1b {
                    Filter::Escape
                } else if (osc && byte == 7) || matches!(byte, 0x18 | 0x1a) {
                    Filter::Ground
                } else {
                    Filter::String { osc }
                }
            }
        };
    }

    fn feed(&mut self, byte: u8) {
        let mut action = ScreenAction::default();
        self.activity_parser.advance(&mut action, &[byte]);
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let (_, col) = screen.cursor_position();
        let alternate = screen.alternate_screen();
        // Observe row differences for erases/scrolls. Ordinary printable text
        // stamps the row even if it redraws identical characters, unlike a hash.
        let needs_diff = action.screen_event
            || action.zero_width
            || (action.printed && col >= cols.saturating_sub(1));
        let cells = usize::from(rows) * usize::from(cols) * 2;
        let before = if needs_diff && self.diff_budget >= cells {
            self.diff_budget -= cells;
            Some(screen.rows(0, cols).collect::<Vec<_>>())
        } else {
            if needs_diff {
                self.row_cursors.fill(0);
            }
            None
        };
        self.parser.process(&[byte]);
        let screen = self.parser.screen();
        if alternate != screen.alternate_screen() {
            self.row_cursors.fill(self.processed);
        }
        if let Some(before) = before {
            for (index, line) in screen.rows(0, cols).enumerate() {
                if before.get(index) != Some(&line) {
                    self.row_cursors[index] = self.processed;
                }
            }
        }
        if action.printed && !action.zero_width {
            self.row_cursors[usize::from(screen.cursor_position().0)] = self.processed;
        }
    }

    pub fn snapshot(
        &self,
        runtime: &str,
        output_cursor: u64,
        max_bytes: usize,
    ) -> TerminalScreenSnapshot {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let mut remaining = max_bytes.min(TERMINAL_SCREEN_MAX_BYTES);
        let mut truncated = false;
        // Include the entire cursor row, not only text before the cursor: a cursor
        // moved into a result row must not turn its prefix into a false prompt.
        let mut cursor_line = screen
            .rows(0, cols)
            .nth(usize::from(cursor_row))
            .unwrap_or_default();
        limit_text(&mut cursor_line, &mut remaining, &mut truncated);
        let mut lines = Vec::new();
        for mut line in screen.rows(0, cols) {
            let before = truncated;
            limit_text(&mut line, &mut remaining, &mut truncated);
            lines.push(line);
            if truncated && !before {
                break;
            }
            if remaining == 0 && lines.len() < usize::from(rows) {
                truncated = true;
                break;
            }
        }
        TerminalScreenSnapshot {
            runtime_id: runtime.into(),
            output_cursor,
            rows,
            cols,
            cursor_row,
            cursor_col,
            cursor_visible: !screen.hide_cursor(),
            cursor_line,
            cursor_line_cursor: self.row_cursors[usize::from(cursor_row)],
            modes: TerminalScreenModes {
                alternate_screen: screen.alternate_screen(),
                application_cursor: screen.application_cursor(),
                application_keypad: screen.application_keypad(),
                bracketed_paste: screen.bracketed_paste(),
            },
            lines,
            truncated,
            size_limited: self.size_limited,
        }
    }
}

// This parser only reports write activity; vt100 remains the sole screen model.
#[derive(Default)]
struct ScreenAction {
    printed: bool,
    zero_width: bool,
    screen_event: bool,
}
impl vte::Perform for ScreenAction {
    fn print(&mut self, c: char) {
        self.printed = c != '\u{fffd}' && !('\u{80}'..'\u{a0}').contains(&c);
        self.zero_width = self.printed && c.width().unwrap_or(1) == 0;
    }
    fn execute(&mut self, byte: u8) {
        self.screen_event = matches!(byte, 10 | 11 | 12);
    }
    fn csi_dispatch(&mut self, _: &vte::Params, _: &[u8], _: bool, action: char) {
        self.screen_event = matches!(action, '@' | 'J' | 'K' | 'L' | 'M' | 'P' | 'S' | 'T' | 'X');
    }
    fn esc_dispatch(&mut self, _: &[u8], _: bool, byte: u8) {
        self.screen_event = matches!(byte, b'M' | b'c');
    }
}

fn limit_text(text: &mut String, remaining: &mut usize, truncated: &mut bool) {
    if text.len() > *remaining {
        let mut end = *remaining;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        *truncated = true;
    }
    *remaining -= text.len();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_tracks_overwrite_wide_text_clear_and_split_escapes() {
        let mut screen = TerminalScreen::new(4, 20);
        screen.process("progress 10%\r\x1b[K中文".as_bytes());
        screen.process(b"\x1b[");
        screen.process(b"31m!\x1b[0m");
        let snapshot = screen.snapshot("r", 42, 1000);
        assert_eq!(snapshot.lines[0], "中文!");
        assert_eq!(snapshot.cursor_col, 5);
        assert_eq!(snapshot.output_cursor, 42);
        screen.process(b"\x1b[2J\x1b[Hnew");
        assert_eq!(screen.snapshot("r", 50, 1000).lines[0], "new");
    }
    #[test]
    fn alternate_screen_modes_and_resize_are_runtime_state() {
        let mut screen = TerminalScreen::new(4, 20);
        screen.process(b"shell\x1b[?1049h\x1b[?1h\x1b[?2004h\x1b[?25l\x1b=\x1b[Hmenu");
        let alt = screen.snapshot("r", 0, 1000);
        assert_eq!(alt.lines[0], "menu");
        assert!(
            alt.modes.alternate_screen
                && alt.modes.application_cursor
                && alt.modes.application_keypad
                && alt.modes.bracketed_paste
        );
        assert!(!alt.cursor_visible);
        screen.resize(6, 30);
        screen.process(b"\x1b[?1049l\x1b[?1l\x1b[?2004l\x1b[?25h\x1b>");
        let normal = screen.snapshot("r", 0, 1000);
        assert_eq!(normal.lines[0], "shell");
        assert_eq!((normal.rows, normal.cols), (6, 30));
        assert!(
            !normal.modes.alternate_screen
                && !normal.modes.application_cursor
                && !normal.modes.application_keypad
                && !normal.modes.bracketed_paste
        );
    }
    #[test]
    fn control_strings_and_response_sizes_are_bounded() {
        let mut screen = TerminalScreen::new(4, 20);
        screen.process(b"a\x1b]52;c;");
        for _ in 0..1024 {
            screen.process(&[b'x'; 8192]);
        }
        screen.process(b"\x1b");
        screen.process(b"\\b\x1bPignored\x1b\\c");
        assert_eq!(screen.snapshot("r", 0, 1000).lines[0], "abc");
        screen.process("中文中文中文".as_bytes());
        let small = screen.snapshot("r", 0, 7);
        assert!(small.truncated);
        assert!(small.lines.iter().map(String::len).sum::<usize>() + small.cursor_line.len() <= 7);
        screen.resize(10000, 10000);
        screen.resize(4, 20);
        assert!(screen.snapshot("r", 0, 1000).size_limited);
    }

    #[test]
    fn c1_control_strings_are_hidden_and_esc_aborts_strings() {
        let mut screen = TerminalScreen::new(4, 80);
        // UTF-8 C1 introducer and terminator split across calls.
        screen.process(b"a\xc2");
        screen.process(b"\x9d52;c;SECRET\xc2");
        screen.process(b"\x9cb");
        screen.process("\u{90}private\u{9c}c".as_bytes());
        screen.process(b"\x1b]0;title\x1b[31md");
        screen.process(b"\x1bPdiscard\x1b[0me");
        assert_eq!(screen.snapshot("r", 0, 1000).lines[0], "abcde");
    }

    #[test]
    fn row_freshness_distinguishes_identical_redraw_from_unrelated_output() {
        let mut screen = TerminalScreen::new(4, 80);
        screen.process(b"mysql> ");
        let initial = screen.snapshot("r", 7, 1000).cursor_line_cursor;
        screen.process(b"\x1b7\x1b[2;1Hbackground\x1b8\x1b]0;title\x07");
        assert_eq!(screen.snapshot("r", 50, 1000).cursor_line_cursor, initial);
        screen.process(b"\rmysql> ");
        assert!(screen.snapshot("r", 60, 1000).cursor_line_cursor > initial);
    }

    #[test]
    fn ignored_replacement_characters_do_not_refresh_a_prompt() {
        let mut screen = TerminalScreen::new(4, 80);
        screen.process(b"mysql> ");
        let before = screen.snapshot("r", 7, 1000);
        screen.process("\u{fffd}".as_bytes());
        let after = screen.snapshot("r", 10, 1000);
        assert_eq!(after.cursor_line, before.cursor_line);
        assert_eq!(after.cursor_line_cursor, before.cursor_line_cursor);
    }

    #[test]
    fn narrow_grid_wide_text_is_safe_and_reports_incomplete_model() {
        let mut screen = TerminalScreen::new(1, 1);
        screen.process("中".as_bytes());
        let snapshot = screen.snapshot("r", 3, 1000);
        assert_eq!(snapshot.cols, 2);
        assert!(snapshot.size_limited);
        assert!(snapshot.lines[0].contains('中'));
    }

    #[test]
    fn zero_width_characters_stamp_only_cells_actually_changed() {
        let mut screen = TerminalScreen::new(4, 80);
        screen.process(b"mysql> \r");
        let before = screen.snapshot("r", 8, 1000).cursor_line_cursor;
        screen.process("\u{301}".as_bytes());
        assert_eq!(screen.snapshot("r", 10, 1000).cursor_line_cursor, before);

        let mut wrapped = TerminalScreen::new(3, 4);
        wrapped.process(b"abcde\r"); // second row, column zero, previous row wrapped
        let previous_stamp = wrapped.row_cursors[0];
        let current_stamp = wrapped.row_cursors[1];
        wrapped.process("\u{301}".as_bytes());
        assert!(wrapped.row_cursors[0] > previous_stamp);
        assert_eq!(wrapped.row_cursors[1], current_stamp);
    }

    #[test]
    fn control_burst_has_bounded_freshness_work_and_recovers_on_new_text() {
        for (rows, cols) in [(24, 80), (256, 512)] {
            let mut screen = TerminalScreen::new(rows, cols);
            let start = std::time::Instant::now();
            screen.process(&[b'\n'; 8192]);
            eprintln!("screen {rows}x{cols}: 8 KiB LF burst {:?}", start.elapsed());
            // At most 262144 cell visits for before/after freshness comparisons,
            // rather than full-screen rendering per control character.
            assert!(screen.diff_budget < (rows * cols * 2) as usize);
            assert_eq!(screen.snapshot("r", 8192, 65536).cursor_line_cursor, 0);
            screen.process(b"mysql> ");
            assert!(screen.snapshot("r", 8199, 65536).cursor_line_cursor > 8192);
        }
    }
}
