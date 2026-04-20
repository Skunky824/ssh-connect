use crate::browser::FileBrowser;
use crate::vterm::{CellAttr, VTerm};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

/// Which pane is focused.
#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Sidebar,
    Terminal,
}

/// Draw the full UI.
pub fn render(
    f: &mut Frame,
    browser: &FileBrowser,
    vterm: &VTerm,
    focus: Focus,
    show_sidebar: bool,
    status_msg: &str,
) {
    let area = f.area();

    // Status bar at bottom (1 row)
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = outer[0];
    let status_area = outer[1];

    // Split main area into sidebar + terminal (or just terminal)
    let (sidebar_area, terminal_area) = if show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(1)])
            .split(main_area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, main_area)
    };

    // Draw sidebar
    if let Some(sb_area) = sidebar_area {
        draw_sidebar(f, sb_area, browser, focus == Focus::Sidebar);
    }

    // Draw terminal
    draw_terminal(f, terminal_area, vterm, focus == Focus::Terminal);

    // Draw status bar
    draw_status_bar(f, status_area, status_msg, focus);
}

fn draw_sidebar(f: &mut Frame, area: Rect, browser: &FileBrowser, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if browser.search_active {
        format!(" {}  /{} ", browser.current_path, browser.search_query)
    } else {
        format!(" {} ", browser.current_path)
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible_height = inner.height as usize;

    // Build list items
    let items: Vec<ListItem> = browser
        .entries
        .iter()
        .enumerate()
        .skip(browser.scroll_offset)
        .take(visible_height)
        .map(|(i, entry)| {
            let icon = if entry.is_dir { "📁 " } else { "📄 " };
            let name = &entry.name;
            let style = if i == browser.selected {
                Style::default()
                    .bg(Color::Indexed(236))
                    .fg(if entry.is_dir {
                        Color::Cyan
                    } else {
                        Color::White
                    })
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(if entry.is_dir {
                    Color::Cyan
                } else {
                    Color::White
                })
            };
            ListItem::new(Line::from(vec![
                Span::styled(icon, style),
                Span::styled(name.to_string(), style),
            ]))
        })
        .collect();

    if let Some(ref err) = browser.error {
        let err_widget = Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::Red));
        f.render_widget(err_widget, inner);
    } else if items.is_empty() {
        let empty = Paragraph::new("  (empty)")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, inner);
    } else {
        let list = List::new(items);
        f.render_widget(list, inner);
    }
}

fn draw_terminal(f: &mut Frame, area: Rect, vterm: &VTerm, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Terminal ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let grid = &vterm.grid;
    let display_rows = (inner.height as usize).min(grid.rows);
    let display_cols = (inner.width as usize).min(grid.cols);

    for row in 0..display_rows {
        let mut spans: Vec<Span> = Vec::new();
        let mut col = 0;

        while col < display_cols {
            let cell = &grid.cells[row][col];
            let style = cell_to_style(&cell.attr);

            // Batch consecutive cells with the same style
            let mut text = String::new();
            while col < display_cols {
                let c = &grid.cells[row][col];
                if cell_to_style(&c.attr) != style {
                    break;
                }
                text.push(c.c);
                col += 1;
            }
            spans.push(Span::styled(text, style));
        }

        let line = Line::from(spans);
        let line_area = Rect::new(inner.x, inner.y + row as u16, inner.width, 1);
        f.render_widget(Paragraph::new(line), line_area);
    }

    // Draw cursor
    if grid.cursor_row < display_rows && grid.cursor_col < display_cols {
        f.set_cursor_position((
            inner.x + grid.cursor_col as u16,
            inner.y + grid.cursor_row as u16,
        ));
    }
}

fn cell_to_style(attr: &CellAttr) -> Style {
    let (fg, bg) = if attr.reverse {
        (attr.bg, attr.fg)
    } else {
        (attr.fg, attr.bg)
    };

    let mut style = Style::default().fg(fg).bg(bg);
    if attr.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if attr.dim {
        style = style.add_modifier(Modifier::DIM);
    }
    if attr.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if attr.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn draw_status_bar(f: &mut Frame, area: Rect, msg: &str, focus: Focus) {
    let focus_str = match focus {
        Focus::Sidebar => "[SIDEBAR]",
        Focus::Terminal => "[TERMINAL]",
    };

    let help = " F2:sidebar  F3:cwd-hook  Ctrl+B:focus  /|Ctrl+F:search  Ctrl+Q:quit";

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", focus_str),
            Style::default().bg(Color::Indexed(236)).fg(Color::Cyan),
        ),
        Span::styled(
            format!(" {} ", msg),
            Style::default().bg(Color::Indexed(236)).fg(Color::White),
        ),
        Span::styled(
            help,
            Style::default()
                .bg(Color::Indexed(236))
                .fg(Color::DarkGray),
        ),
    ]);

    let bar = Paragraph::new(line).style(Style::default().bg(Color::Indexed(236)));
    f.render_widget(bar, area);
}
