use ratatui::style::Color;

/// Attributes for a single terminal cell.
#[derive(Clone, Copy, Debug)]
pub struct CellAttr {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Default for CellAttr {
    fn default() -> Self {
        CellAttr {
            fg: Color::Reset,
            bg: Color::Reset,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            reverse: false,
        }
    }
}

/// A single character cell in the terminal grid.
#[derive(Clone, Debug)]
pub struct Cell {
    pub c: char,
    pub attr: CellAttr,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            c: ' ',
            attr: CellAttr::default(),
        }
    }
}

/// The virtual terminal grid that processes VT escape sequences.
pub struct VTerm {
    parser: vte::Parser,
    pub grid: Grid,
}

pub struct Grid {
    pub cells: Vec<Vec<Cell>>,
    pub rows: usize,
    pub cols: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
    saved_cursor: Option<(usize, usize)>,
    current_attr: CellAttr,
    scroll_top: usize,
    scroll_bottom: usize,
    /// CWD detected from OSC 7 sequences.
    pub cwd: Option<String>,
    /// Whether CWD changed since last check.
    pub cwd_changed: bool,
    // Alternate screen buffer
    alt_cells: Option<Vec<Vec<Cell>>>,
    alt_cursor: Option<(usize, usize)>,
}

impl VTerm {
    pub fn new(rows: usize, cols: usize) -> Self {
        VTerm {
            parser: vte::Parser::new(),
            grid: Grid::new(rows, cols),
        }
    }

    /// Feed raw bytes from the SSH channel into the terminal emulator.
    pub fn process(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.parser.advance(&mut self.grid, byte);
        }
    }

    /// Resize the terminal grid (preserves content where possible).
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.grid.resize(rows, cols);
    }

    /// Take the CWD if it changed.
    pub fn take_cwd(&mut self) -> Option<String> {
        if self.grid.cwd_changed {
            self.grid.cwd_changed = false;
            self.grid.cwd.clone()
        } else {
            None
        }
    }

    /// Fallback CWD detection from visible prompt text.
    ///
    /// Useful when OSC 7 hooks are not active (e.g. after `sudo -i`).
    pub fn detect_cwd_from_prompt(&self) -> Option<String> {
        self.grid.detect_cwd_from_prompt()
    }
}

impl Grid {
    fn new(rows: usize, cols: usize) -> Self {
        Grid {
            cells: vec![vec![Cell::default(); cols]; rows],
            rows,
            cols,
            cursor_row: 0,
            cursor_col: 0,
            saved_cursor: None,
            current_attr: CellAttr::default(),
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            cwd: None,
            cwd_changed: false,
            alt_cells: None,
            alt_cursor: None,
        }
    }

    fn resize(&mut self, rows: usize, cols: usize) {
        let mut new_cells = vec![vec![Cell::default(); cols]; rows];
        let copy_rows = rows.min(self.rows);
        let copy_cols = cols.min(self.cols);
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                new_cells[r][c] = self.cells[r][c].clone();
            }
        }
        self.cells = new_cells;
        self.rows = rows;
        self.cols = cols;
        self.scroll_top = 0;
        self.scroll_bottom = rows.saturating_sub(1);
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
    }

    fn detect_cwd_from_prompt(&self) -> Option<String> {
        // Scan from bottom to top for the most recent shell-like prompt.
        for row in (0..self.rows).rev() {
            let line: String = self.cells[row].iter().map(|cell| cell.c).collect();
            if let Some(path) = parse_prompt_cwd(line.trim()) {
                return Some(path);
            }
        }
        None
    }

    fn scroll_up(&mut self) {
        if self.scroll_top < self.scroll_bottom && self.scroll_bottom < self.rows {
            self.cells.remove(self.scroll_top);
            self.cells
                .insert(self.scroll_bottom, vec![Cell::default(); self.cols]);
        }
    }

    fn scroll_down(&mut self) {
        if self.scroll_top < self.scroll_bottom && self.scroll_bottom < self.rows {
            self.cells.remove(self.scroll_bottom);
            self.cells
                .insert(self.scroll_top, vec![Cell::default(); self.cols]);
        }
    }

    fn put_char(&mut self, c: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.new_line();
        }
        if self.cursor_row < self.rows && self.cursor_col < self.cols {
            self.cells[self.cursor_row][self.cursor_col] = Cell {
                c,
                attr: self.current_attr,
            };
            self.cursor_col += 1;
        }
    }

    fn new_line(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up();
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }

    fn enter_alt_screen(&mut self) {
        let saved = self.cells.clone();
        let cursor = (self.cursor_row, self.cursor_col);
        self.alt_cells = Some(saved);
        self.alt_cursor = Some(cursor);
        self.cells = vec![vec![Cell::default(); self.cols]; self.rows];
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    fn exit_alt_screen(&mut self) {
        if let Some(cells) = self.alt_cells.take() {
            self.cells = cells;
            if let Some((r, c)) = self.alt_cursor.take() {
                self.cursor_row = r.min(self.rows.saturating_sub(1));
                self.cursor_col = c.min(self.cols.saturating_sub(1));
            }
        }
    }
}

fn parse_prompt_cwd(line: &str) -> Option<String> {
    // Common prompts:
    // user@host:/path$ 
    // root@host:/path#
    // [user@host path]$  (ignored here unless absolute)
    let end = line.chars().last()?;
    if end != '$' && end != '#' && end != '%' {
        return None;
    }

    // Take the segment before the final prompt marker.
    let prefix = line[..line.len().saturating_sub(end.len_utf8())].trim_end();

    // Find the final ':' and parse path after it.
    // Example: root@rocky:/tmp or root@rocky:~
    if let Some(colon) = prefix.rfind(':') {
        let user_host = prefix[..colon].trim();
        let user = user_host.split('@').next().unwrap_or("").trim();
        let candidate = prefix[colon + 1..].trim();
        if candidate.starts_with('/') {
            return Some(candidate.to_string());
        }

        // Expand common tilde forms that appear after sudo -i prompts.
        if candidate == "~" {
            if user == "root" {
                return Some("/root".to_string());
            }
            if !user.is_empty() {
                return Some(format!("/home/{}", user));
            }
        }

        if let Some(rest) = candidate.strip_prefix("~/") {
            if user == "root" {
                return Some(format!("/root/{}", rest));
            }
            if !user.is_empty() {
                return Some(format!("/home/{}/{}", user, rest));
            }
        }
    }

    None
}

impl vte::Perform for Grid {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // Backspace
            0x08 => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            // Tab
            0x09 => {
                let next_tab = (self.cursor_col + 8) & !7;
                self.cursor_col = next_tab.min(self.cols - 1);
            }
            // Line feed / Vertical tab / Form feed
            0x0A | 0x0B | 0x0C => {
                self.new_line();
            }
            // Carriage return
            0x0D => {
                self.cursor_col = 0;
            }
            // Bell
            0x07 => {}
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // OSC 7 ; <cwd> ST — reports current working directory
        if params.len() >= 2 && params[0] == b"7" {
            if let Ok(cwd_str) = std::str::from_utf8(params[1]) {
                // May be file://hostname/path or just /path
                let path = if let Some(rest) = cwd_str.strip_prefix("file://") {
                    // Strip hostname part
                    if let Some(slash_pos) = rest.find('/') {
                        &rest[slash_pos..]
                    } else {
                        rest
                    }
                } else {
                    cwd_str
                };
                let path = path.to_string();
                if self.cwd.as_deref() != Some(&path) {
                    self.cwd = Some(path);
                    self.cwd_changed = true;
                }
            }
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let ps: Vec<u16> = params.iter().flat_map(|sub| sub.iter().copied()).collect();

        let param = |idx: usize, default: u16| -> u16 {
            ps.get(idx).copied().filter(|&v| v != 0).unwrap_or(default)
        };

        match action {
            // Cursor Up
            'A' => {
                let n = param(0, 1) as usize;
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            // Cursor Down
            'B' => {
                let n = param(0, 1) as usize;
                self.cursor_row = (self.cursor_row + n).min(self.rows - 1);
            }
            // Cursor Forward
            'C' => {
                let n = param(0, 1) as usize;
                self.cursor_col = (self.cursor_col + n).min(self.cols - 1);
            }
            // Cursor Backward
            'D' => {
                let n = param(0, 1) as usize;
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            // Cursor Next Line
            'E' => {
                let n = param(0, 1) as usize;
                self.cursor_row = (self.cursor_row + n).min(self.rows - 1);
                self.cursor_col = 0;
            }
            // Cursor Previous Line
            'F' => {
                let n = param(0, 1) as usize;
                self.cursor_row = self.cursor_row.saturating_sub(n);
                self.cursor_col = 0;
            }
            // Cursor Horizontal Absolute
            'G' => {
                let col = param(0, 1) as usize;
                self.cursor_col = (col - 1).min(self.cols - 1);
            }
            // Cursor Position
            'H' | 'f' => {
                let row = param(0, 1) as usize;
                let col = param(1, 1) as usize;
                self.cursor_row = (row - 1).min(self.rows - 1);
                self.cursor_col = (col - 1).min(self.cols - 1);
            }
            // Erase in Display
            'J' => {
                let mode = param(0, 0);
                match mode {
                    0 => {
                        // Clear from cursor to end
                        for c in self.cursor_col..self.cols {
                            self.cells[self.cursor_row][c] = Cell::default();
                        }
                        for r in (self.cursor_row + 1)..self.rows {
                            for c in 0..self.cols {
                                self.cells[r][c] = Cell::default();
                            }
                        }
                    }
                    1 => {
                        // Clear from start to cursor
                        for r in 0..self.cursor_row {
                            for c in 0..self.cols {
                                self.cells[r][c] = Cell::default();
                            }
                        }
                        for c in 0..=self.cursor_col.min(self.cols - 1) {
                            self.cells[self.cursor_row][c] = Cell::default();
                        }
                    }
                    2 | 3 => {
                        // Clear entire screen
                        for r in 0..self.rows {
                            for c in 0..self.cols {
                                self.cells[r][c] = Cell::default();
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Erase in Line
            'K' => {
                let mode = param(0, 0);
                match mode {
                    0 => {
                        for c in self.cursor_col..self.cols {
                            self.cells[self.cursor_row][c] = Cell::default();
                        }
                    }
                    1 => {
                        for c in 0..=self.cursor_col.min(self.cols - 1) {
                            self.cells[self.cursor_row][c] = Cell::default();
                        }
                    }
                    2 => {
                        for c in 0..self.cols {
                            self.cells[self.cursor_row][c] = Cell::default();
                        }
                    }
                    _ => {}
                }
            }
            // Scroll Up
            'S' => {
                let n = param(0, 1) as usize;
                for _ in 0..n {
                    self.scroll_up();
                }
            }
            // Scroll Down
            'T' => {
                let n = param(0, 1) as usize;
                for _ in 0..n {
                    self.scroll_down();
                }
            }
            // Insert Lines
            'L' => {
                let n = param(0, 1) as usize;
                for _ in 0..n {
                    if self.cursor_row <= self.scroll_bottom {
                        self.cells.remove(self.scroll_bottom);
                        self.cells
                            .insert(self.cursor_row, vec![Cell::default(); self.cols]);
                    }
                }
            }
            // Delete Lines
            'M' => {
                let n = param(0, 1) as usize;
                for _ in 0..n {
                    if self.cursor_row <= self.scroll_bottom {
                        self.cells.remove(self.cursor_row);
                        self.cells
                            .insert(self.scroll_bottom, vec![Cell::default(); self.cols]);
                    }
                }
            }
            // Delete Characters
            'P' => {
                let n = param(0, 1) as usize;
                let row = self.cursor_row;
                let col = self.cursor_col;
                for _ in 0..n {
                    if col < self.cols {
                        self.cells[row].remove(col);
                        self.cells[row].push(Cell::default());
                    }
                }
            }
            // Insert Characters
            '@' => {
                let n = param(0, 1) as usize;
                let row = self.cursor_row;
                let col = self.cursor_col;
                for _ in 0..n {
                    if col < self.cols {
                        self.cells[row].insert(col, Cell::default());
                        self.cells[row].truncate(self.cols);
                    }
                }
            }
            // SGR - Select Graphic Rendition
            'm' => {
                self.apply_sgr(&ps);
            }
            // Set scrolling region
            'r' => {
                let top = param(0, 1) as usize;
                let bottom = param(1, self.rows as u16) as usize;
                self.scroll_top = (top - 1).min(self.rows - 1);
                self.scroll_bottom = (bottom - 1).min(self.rows - 1);
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            // Erase Characters
            'X' => {
                let n = param(0, 1) as usize;
                for i in 0..n {
                    let c = self.cursor_col + i;
                    if c < self.cols {
                        self.cells[self.cursor_row][c] = Cell::default();
                    }
                }
            }
            // Save cursor position
            's' => {
                self.saved_cursor = Some((self.cursor_row, self.cursor_col));
            }
            // Restore cursor position
            'u' => {
                if let Some((r, c)) = self.saved_cursor {
                    self.cursor_row = r.min(self.rows - 1);
                    self.cursor_col = c.min(self.cols - 1);
                }
            }
            // Cursor Visibility (DECTCEM) and alt screen
            'h' | 'l' => {
                if intermediates == b"?" {
                    for &p in &ps {
                        match p {
                            // Alternate screen buffer
                            1049 | 47 | 1047 => {
                                if action == 'h' {
                                    self.enter_alt_screen();
                                } else {
                                    self.exit_alt_screen();
                                }
                            }
                            // Other DEC private modes (cursor visibility etc) - ignore for now
                            _ => {}
                        }
                    }
                }
            }
            // Tab clear, etc. - ignore
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            // Save cursor (DECSC)
            b'7' => {
                self.saved_cursor = Some((self.cursor_row, self.cursor_col));
            }
            // Restore cursor (DECRC)
            b'8' => {
                if let Some((r, c)) = self.saved_cursor {
                    self.cursor_row = r.min(self.rows - 1);
                    self.cursor_col = c.min(self.cols - 1);
                }
            }
            // Reverse Index (scroll down)
            b'M' => {
                if self.cursor_row == self.scroll_top {
                    self.scroll_down();
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                }
            }
            // Index (scroll up)
            b'D' => {
                if self.cursor_row == self.scroll_bottom {
                    self.scroll_up();
                } else if self.cursor_row < self.rows - 1 {
                    self.cursor_row += 1;
                }
            }
            // Next line
            b'E' => {
                self.cursor_col = 0;
                if self.cursor_row == self.scroll_bottom {
                    self.scroll_up();
                } else if self.cursor_row < self.rows - 1 {
                    self.cursor_row += 1;
                }
            }
            // Reset
            b'c' => {
                let (rows, cols) = (self.rows, self.cols);
                *self = Grid::new(rows, cols);
            }
            _ => {
                // Handle charset designations etc. — ignore
                let _ = intermediates;
            }
        }
    }
}

impl Grid {
    fn apply_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.current_attr = CellAttr::default();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.current_attr = CellAttr::default(),
                1 => self.current_attr.bold = true,
                2 => self.current_attr.dim = true,
                3 => self.current_attr.italic = true,
                4 => self.current_attr.underline = true,
                7 => self.current_attr.reverse = true,
                21 | 22 => {
                    self.current_attr.bold = false;
                    self.current_attr.dim = false;
                }
                23 => self.current_attr.italic = false,
                24 => self.current_attr.underline = false,
                27 => self.current_attr.reverse = false,
                // Standard foreground colors
                30 => self.current_attr.fg = Color::Black,
                31 => self.current_attr.fg = Color::Red,
                32 => self.current_attr.fg = Color::Green,
                33 => self.current_attr.fg = Color::Yellow,
                34 => self.current_attr.fg = Color::Blue,
                35 => self.current_attr.fg = Color::Magenta,
                36 => self.current_attr.fg = Color::Cyan,
                37 => self.current_attr.fg = Color::White,
                // 256/truecolor foreground
                38 => {
                    i += 1;
                    if i < params.len() {
                        match params[i] {
                            5 => {
                                i += 1;
                                if i < params.len() {
                                    self.current_attr.fg = Color::Indexed(params[i] as u8);
                                }
                            }
                            2 => {
                                if i + 3 < params.len() {
                                    let r = params[i + 1] as u8;
                                    let g = params[i + 2] as u8;
                                    let b = params[i + 3] as u8;
                                    self.current_attr.fg = Color::Rgb(r, g, b);
                                    i += 3;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                39 => self.current_attr.fg = Color::Reset,
                // Standard background colors
                40 => self.current_attr.bg = Color::Black,
                41 => self.current_attr.bg = Color::Red,
                42 => self.current_attr.bg = Color::Green,
                43 => self.current_attr.bg = Color::Yellow,
                44 => self.current_attr.bg = Color::Blue,
                45 => self.current_attr.bg = Color::Magenta,
                46 => self.current_attr.bg = Color::Cyan,
                47 => self.current_attr.bg = Color::White,
                // 256/truecolor background
                48 => {
                    i += 1;
                    if i < params.len() {
                        match params[i] {
                            5 => {
                                i += 1;
                                if i < params.len() {
                                    self.current_attr.bg = Color::Indexed(params[i] as u8);
                                }
                            }
                            2 => {
                                if i + 3 < params.len() {
                                    let r = params[i + 1] as u8;
                                    let g = params[i + 2] as u8;
                                    let b = params[i + 3] as u8;
                                    self.current_attr.bg = Color::Rgb(r, g, b);
                                    i += 3;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                49 => self.current_attr.bg = Color::Reset,
                // Bright foreground
                90 => self.current_attr.fg = Color::DarkGray,
                91 => self.current_attr.fg = Color::LightRed,
                92 => self.current_attr.fg = Color::LightGreen,
                93 => self.current_attr.fg = Color::LightYellow,
                94 => self.current_attr.fg = Color::LightBlue,
                95 => self.current_attr.fg = Color::LightMagenta,
                96 => self.current_attr.fg = Color::LightCyan,
                97 => self.current_attr.fg = Color::Gray,
                // Bright background
                100 => self.current_attr.bg = Color::DarkGray,
                101 => self.current_attr.bg = Color::LightRed,
                102 => self.current_attr.bg = Color::LightGreen,
                103 => self.current_attr.bg = Color::LightYellow,
                104 => self.current_attr.bg = Color::LightBlue,
                105 => self.current_attr.bg = Color::LightMagenta,
                106 => self.current_attr.bg = Color::LightCyan,
                107 => self.current_attr.bg = Color::Gray,
                _ => {}
            }
            i += 1;
        }
    }
}
