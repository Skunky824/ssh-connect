use crate::ssh::{DirEntry, SshSession};

/// Remote file browser state.
pub struct FileBrowser {
    /// Current remote directory path.
    pub current_path: String,
    /// Directory entries (sorted: dirs first, then alphabetical).
    pub entries: Vec<DirEntry>,
    /// Currently selected index.
    pub selected: usize,
    /// Scroll offset for display.
    pub scroll_offset: usize,
    /// Whether a refresh is needed.
    pub needs_refresh: bool,
    /// Error message from last operation.
    pub error: Option<String>,
    /// Whether incremental search is active in sidebar.
    pub search_active: bool,
    /// Current incremental search query.
    pub search_query: String,
}

impl FileBrowser {
    pub fn new() -> Self {
        FileBrowser {
            current_path: String::from("/"),
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            needs_refresh: true,
            error: None,
            search_active: false,
            search_query: String::new(),
        }
    }

    /// Refresh the file listing from the remote server.
    pub async fn refresh(&mut self, ssh: &SshSession) {
        match ssh.list_dir(&self.current_path).await {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
                // Clamp selection
                if self.entries.is_empty() {
                    self.selected = 0;
                } else if self.selected >= self.entries.len() {
                    self.selected = self.entries.len() - 1;
                }
                self.needs_refresh = false;
            }
            Err(e) => {
                self.error = Some(format!("{:#}", e));
                self.entries.clear();
                self.selected = 0;
                self.needs_refresh = false;
            }
        }
    }

    /// Navigate to a new directory.
    pub fn set_path(&mut self, path: String) {
        if path != self.current_path {
            self.current_path = path;
            self.selected = 0;
            self.scroll_offset = 0;
            self.needs_refresh = true;
            self.search_active = false;
            self.search_query.clear();
        }
    }

    /// Start incremental search mode.
    pub fn start_search(&mut self) {
        self.search_active = true;
        self.search_query.clear();
    }

    /// Leave incremental search mode and clear query.
    pub fn cancel_search(&mut self) {
        self.search_active = false;
        self.search_query.clear();
    }

    /// Leave incremental search mode keeping current selection.
    pub fn finish_search(&mut self) {
        self.search_active = false;
    }

    /// Add one character to the active search query and move to first match.
    pub fn push_search_char(&mut self, c: char) {
        self.search_query.push(c);
        self.select_first_match();
    }

    /// Remove one character from search query and update match.
    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        self.select_first_match();
    }

    fn select_first_match(&mut self) {
        if self.search_query.is_empty() {
            return;
        }

        let needle = self.search_query.to_lowercase();
        if let Some(idx) = self
            .entries
            .iter()
            .position(|e| e.name.to_lowercase().contains(&needle))
        {
            self.selected = idx;
        }
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        if !self.entries.is_empty() && self.selected < self.entries.len() - 1 {
            self.selected += 1;
        }
    }

    /// Get the currently selected entry.
    pub fn selected_entry(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }

    /// Enter the selected directory, or return the file path for editing.
    pub fn enter(&mut self) -> Option<EntryAction> {
        let entry = self.entries.get(self.selected)?.clone();
        let full_path = if self.current_path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", self.current_path, entry.name)
        };

        if entry.is_dir {
            self.current_path = full_path;
            self.selected = 0;
            self.scroll_offset = 0;
            self.needs_refresh = true;
            Some(EntryAction::EnterDir)
        } else {
            Some(EntryAction::OpenFile(full_path))
        }
    }

    /// Go to the parent directory.
    pub fn go_parent(&mut self) {
        if self.current_path == "/" {
            return;
        }
        if let Some(pos) = self.current_path.rfind('/') {
            let parent = if pos == 0 {
                "/".to_string()
            } else {
                self.current_path[..pos].to_string()
            };
            self.current_path = parent;
            self.selected = 0;
            self.scroll_offset = 0;
            self.needs_refresh = true;
        }
    }

    /// Ensure scroll_offset keeps the selection visible within `visible_height` rows.
    pub fn adjust_scroll(&mut self, visible_height: usize) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected - visible_height + 1;
        }
    }
}

#[derive(Debug)]
pub enum EntryAction {
    EnterDir,
    OpenFile(String),
}
