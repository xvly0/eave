use std::cell::{Cell, RefCell};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;

use windows::core::{implement, w, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HANDLE,
    HGLOBAL, HWND, LPARAM, POINTL, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows::Win32::System::Com::{
    IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, OleInitialize,
    OleUninitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, CF_UNICODETEXT,
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, DragQueryFileW, ILFree, IShellItem, SHCreateItemFromParsingName,
    SHCreateShellItemArrayFromIDLists, SHGetFileInfoW, SHGetIDListFromObject, SHGetImageList,
    ShellExecuteW, HDROP, SHFILEINFOW, SHGFI_FLAGS, SHGFI_ICON, SHGFI_LARGEICON,
    SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHIL_EXTRALARGE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GetIconInfo, PostMessageW, HICON, ICONINFO, SW_SHOWNORMAL, WM_NULL,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ShelfItem {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    #[allow(dead_code)]
    pub icon_width: u32,
    #[allow(dead_code)]
    pub icon_height: u32,
    #[allow(dead_code)]
    pub icon_bgra: Option<Vec<u8>>,
    pub slot_x_anim: f32,
    pub removing: bool,
    pub remove_anim: f32,
}

impl ShelfItem {
    pub fn from_path(path: impl Into<String>) -> Self {
        let cleaned = clean_path_str(&path.into());
        let p = Path::new(&cleaned);
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cleaned.clone());
        let (is_dir, size_bytes) = match fs::metadata(p) {
            Ok(meta) => {
                let dir = meta.is_dir();
                (dir, if dir { 0 } else { meta.len() })
            }
            Err(_) => (false, 0),
        };
        let mut item = Self {
            path: cleaned,
            name,
            is_dir,
            size_bytes,
            icon_width: 0,
            icon_height: 0,
            icon_bgra: None,
            slot_x_anim: -1.0,
            removing: false,
            remove_anim: 0.0,
        };
        item.ensure_icon();
        item
    }

    /// Loads and caches the native Windows Shell icon as premultiplied BGRA8 pixels.
    pub fn ensure_icon(&mut self) {
        if self.icon_bgra.is_some() {
            return;
        }
        if let Some((w, h, bgra)) = load_windows_shell_icon(&self.path, self.is_dir) {
            self.icon_width = w;
            self.icon_height = h;
            self.icon_bgra = Some(bgra);
        }
    }

    pub fn format_size(&self) -> String {
        if self.is_dir {
            return "Folder".to_string();
        }
        const KB: f64 = 1024.0;
        const MB: f64 = 1024.0 * 1024.0;
        const GB: f64 = 1024.0 * 1024.0 * 1024.0;
        let b = self.size_bytes as f64;
        if b < KB {
            format!("{} B", self.size_bytes)
        } else if b < MB {
            let v = b / KB;
            if v >= 10.0 {
                format!("{v:.0} KB")
            } else {
                format!("{v:.1} KB")
            }
        } else if b < GB {
            let v = b / MB;
            if v >= 10.0 {
                format!("{v:.0} MB")
            } else {
                format!("{v:.1} MB")
            }
        } else {
            let v = b / GB;
            if v >= 10.0 {
                format!("{v:.0} GB")
            } else {
                format!("{v:.1} GB")
            }
        }
    }

    #[allow(dead_code)]
    pub fn extension_label(&self) -> String {
        if self.is_dir {
            return "DIR".to_string();
        }
        Path::new(&self.path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.chars().take(4).collect::<String>().to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "FILE".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShelfTab {
    #[default]
    Files,
    Clipboard,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardItem {
    pub text: String,
    pub time_label: String,
    pub is_new: bool,
    pub slot_x_anim: f32,
    pub enter_anim: f32,
    pub removing: bool,
    pub remove_anim: f32,
}

fn default_clipboard_items() -> Vec<ClipboardItem> {
    vec![
        ClipboardItem {
            text: "https://eave.app/releases/v0.1".to_string(),
            time_label: "now".to_string(),
            is_new: true,
            slot_x_anim: 0.0,
            enter_anim: 1.0,
            removing: false,
            remove_anim: 0.0,
        },
        ClipboardItem {
            text: "rgba(135, 195, 255, 0.92)".to_string(),
            time_label: "2 min ago".to_string(),
            is_new: false,
            slot_x_anim: 1.0,
            enter_anim: 1.0,
            removing: false,
            remove_anim: 0.0,
        },
        ClipboardItem {
            text: "cargo build --release --target x86_64".to_string(),
            time_label: "8 min ago".to_string(),
            is_new: false,
            slot_x_anim: 2.0,
            enter_anim: 1.0,
            removing: false,
            remove_anim: 0.0,
        },
        ClipboardItem {
            text: "Release notes — v1.3 shipped".to_string(),
            time_label: "22 min ago".to_string(),
            is_new: false,
            slot_x_anim: 3.0,
            enter_anim: 1.0,
            removing: false,
            remove_anim: 0.0,
        },
    ]
}

#[derive(Clone, Debug)]
pub struct ShelfStore {
    pub items: Vec<ShelfItem>,
    pub is_drag_over: bool,
    pub selected_index: Option<usize>,
    #[allow(dead_code)]
    pub select_mode: bool,
    #[allow(dead_code)]
    pub selected_indices: Vec<usize>,
    pub active_tab: ShelfTab,
    pub clipboard_items: Vec<ClipboardItem>,
    pub last_clipboard_seq: u32,
    pub copied_feedback: Option<(usize, Instant)>,
    pub files_scroll_offset: usize,
    pub files_scroll_anim: f32,
    pub clip_scroll_offset: usize,
    pub clip_scroll_anim: f32,
    pub drag_cursor_screen_px: Option<(i32, i32)>,
    pub pending_drop_screen_px: Option<(i32, i32)>,
    storage_path: PathBuf,
    dirty: bool,
}

impl Default for ShelfStore {
    fn default() -> Self {
        Self::load_from(Self::resolve_path())
    }
}

impl ShelfStore {
    pub fn resolve_path() -> PathBuf {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("eave").join("shelf.txt");
        }
        PathBuf::from("shelf.txt")
    }

    pub fn load_or_create() -> Self {
        let path = Self::resolve_path();
        let seq = unsafe { GetClipboardSequenceNumber() };
        if let Ok(contents) = fs::read_to_string(&path) {
            return Self {
                items: Self::parse_str(&contents),
                is_drag_over: false,
                selected_index: None,
                select_mode: false,
                selected_indices: Vec::new(),
                active_tab: ShelfTab::Files,
                clipboard_items: Vec::new(),
                last_clipboard_seq: seq,
                copied_feedback: None,
                files_scroll_offset: 0,
                files_scroll_anim: 0.0,
                clip_scroll_offset: 0,
                clip_scroll_anim: 0.0,
                drag_cursor_screen_px: None,
                pending_drop_screen_px: None,
                storage_path: path,
                dirty: false,
            };
        }
        let store = Self {
            items: Vec::new(),
            is_drag_over: false,
            selected_index: None,
            select_mode: false,
            selected_indices: Vec::new(),
            active_tab: ShelfTab::Files,
            clipboard_items: Vec::new(),
            last_clipboard_seq: seq,
            copied_feedback: None,
            files_scroll_offset: 0,
            files_scroll_anim: 0.0,
            clip_scroll_offset: 0,
            clip_scroll_anim: 0.0,
            drag_cursor_screen_px: None,
            pending_drop_screen_px: None,
            storage_path: path,
            dirty: false,
        };
        let _ = store.save();
        store
    }

    pub fn load_from(path: impl Into<PathBuf>) -> Self {
        let storage_path = path.into();
        let items = fs::read_to_string(&storage_path)
            .map(|c| Self::parse_str(&c))
            .unwrap_or_default();
        let seq = unsafe { GetClipboardSequenceNumber() };
        Self {
            items,
            is_drag_over: false,
            selected_index: None,
            select_mode: false,
            selected_indices: Vec::new(),
            active_tab: ShelfTab::Files,
            clipboard_items: Vec::new(),
            last_clipboard_seq: seq,
            copied_feedback: None,
            files_scroll_offset: 0,
            files_scroll_anim: 0.0,
            clip_scroll_offset: 0,
            clip_scroll_anim: 0.0,
            drag_cursor_screen_px: None,
            pending_drop_screen_px: None,
            storage_path,
            dirty: false,
        }
    }

    pub fn seed_demo_clipboard(&mut self) {
        self.clipboard_items = default_clipboard_items();
    }

    pub fn parse_str(contents: &str) -> Vec<ShelfItem> {
        let mut items: Vec<ShelfItem> = Vec::new();
        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let item = if let Some((kind, rest)) = line.split_once('\t') {
                if let Some((size_str, path_part)) = rest.split_once('\t') {
                    let path = clean_path_str(path_part);
                    if path.is_empty() {
                        continue;
                    }
                    let mut parsed = ShelfItem::from_path(path);
                    let declared_dir = kind.eq_ignore_ascii_case("D");
                    if parsed.is_dir != declared_dir {
                        parsed.is_dir = declared_dir;
                        parsed.icon_bgra = None;
                        parsed.ensure_icon();
                    }
                    if let Ok(sz) = size_str.trim().parse::<u64>() {
                        if parsed.size_bytes == 0 {
                            parsed.size_bytes = sz;
                        }
                    }
                    parsed
                } else {
                    ShelfItem::from_path(line)
                }
            } else {
                ShelfItem::from_path(line)
            };
            if item.path.is_empty() {
                continue;
            }
            if !contains_path(&items, &item.path) {
                items.push(item);
            }
        }
        items
    }

    pub fn to_storage_string(&self) -> String {
        let mut out = String::from("# Eave shelf\n");
        for item in &self.items {
            let kind = if item.is_dir { "D" } else { "F" };
            let safe_path = sanitize_single_line(&item.path);
            out.push_str(&format!("{kind}\t{}\t{safe_path}\n", item.size_bytes));
        }
        out
    }

    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&self.storage_path, self.to_storage_string())
    }

    pub fn add_paths(&mut self, paths: &[String]) -> usize {
        let mut added = 0usize;
        for raw in paths {
            let cleaned = clean_path_str(raw);
            if cleaned.is_empty() || contains_path(&self.items, &cleaned) {
                continue;
            }
            self.items.push(ShelfItem::from_path(cleaned));
            added += 1;
        }
        if added > 0 {
            self.dirty = true;
            let _ = self.save();
        }
        added
    }

    #[allow(dead_code)]
    pub fn remove_at(&mut self, idx: usize) -> Option<ShelfItem> {
        if idx >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(idx);
        self.selected_index = match self.selected_index {
            Some(sel) if sel == idx => None,
            Some(sel) if sel > idx => Some(sel - 1),
            other => other,
        };
        self.selected_indices.retain(|&i| i != idx);
        for i in &mut self.selected_indices {
            if *i > idx {
                *i -= 1;
            }
        }
        if self.items.is_empty() {
            self.select_mode = false;
        }
        self.dirty = true;
        let _ = self.save();
        Some(removed)
    }

    pub fn clear(&mut self) {
        if self.items.is_empty()
            && self.selected_index.is_none()
            && self.selected_indices.is_empty()
            && !self.select_mode
        {
            return;
        }
        self.items.clear();
        self.selected_index = None;
        self.selected_indices.clear();
        self.select_mode = false;
        self.dirty = true;
        let _ = self.save();
    }

    #[allow(dead_code)]
    pub fn toggle_select_mode(&mut self) {
        self.select_mode = !self.select_mode;
        if !self.select_mode {
            self.selected_indices.clear();
        }
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub fn toggle_item_selected(&mut self, idx: usize) {
        if idx >= self.items.len() {
            return;
        }
        if let Some(pos) = self.selected_indices.iter().position(|&i| i == idx) {
            self.selected_indices.remove(pos);
        } else {
            self.selected_indices.push(idx);
            self.selected_indices.sort_unstable();
        }
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub fn is_item_selected(&self, idx: usize) -> bool {
        self.selected_indices.contains(&idx)
    }

    #[allow(dead_code)]
    pub fn drag_paths_for_item(&self, idx: usize) -> Vec<String> {
        if self.select_mode && self.is_item_selected(idx) && !self.selected_indices.is_empty() {
            self.selected_indices
                .iter()
                .filter_map(|&i| self.items.get(i).map(|it| it.path.clone()))
                .collect()
        } else if let Some(item) = self.items.get(idx) {
            vec![item.path.clone()]
        } else {
            Vec::new()
        }
    }

    pub fn open_item(&self, idx: usize) -> bool {
        self.items
            .get(idx)
            .map(|item| open_path(&item.path))
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn copy_path(&self, idx: usize) -> bool {
        self.items
            .get(idx)
            .map(|item| copy_path_to_clipboard(&item.path))
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn paths(&self) -> Vec<String> {
        self.items.iter().map(|it| it.path.clone()).collect()
    }

    pub fn set_drag_over(&mut self, over: bool) {
        if self.is_drag_over != over {
            self.is_drag_over = over;
            self.dirty = true;
        }
    }

    pub fn take_dirty(&mut self) -> bool {
        let d = self.dirty;
        self.dirty = false;
        d
    }

    pub fn take_drop_screen_px(&mut self) -> Option<(i32, i32)> {
        self.pending_drop_screen_px.take()
    }

    pub fn poll_clipboard(&mut self) {
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq == self.last_clipboard_seq {
            return;
        }
        self.last_clipboard_seq = seq;
        if let Some(text) = read_clipboard_unicode_text() {
            self.push_clipboard_text(&text);
        }
    }

    pub fn push_clipboard_text(&mut self, raw: &str) -> bool {
        let text = raw.trim();
        if text.is_empty() {
            return false;
        }
        if self
            .clipboard_items
            .first()
            .map(|it| it.text.as_str())
            == Some(text)
        {
            return false;
        }
        for item in &mut self.clipboard_items {
            item.is_new = false;
        }
        self.clipboard_items.insert(
            0,
            ClipboardItem {
                text: text.to_string(),
                time_label: "now".to_string(),
                is_new: true,
                slot_x_anim: 0.0,
                enter_anim: 0.0,
                removing: false,
                remove_anim: 0.0,
            },
        );
        self.clipboard_items.truncate(24);
        self.copied_feedback = None;
        self.clamp_scrolls();
        self.dirty = true;
        true
    }

    pub fn start_remove_clip_at(&mut self, active_idx: usize) -> bool {
        let mut seen = 0usize;
        for item in &mut self.clipboard_items {
            if item.removing {
                continue;
            }
            if seen == active_idx {
                item.removing = true;
                self.dirty = true;
                return true;
            }
            seen += 1;
        }
        false
    }

    pub fn start_remove_file_at(&mut self, active_idx: usize) -> bool {
        let mut seen = 0usize;
        for (i, item) in self.items.iter_mut().enumerate() {
            if item.removing {
                continue;
            }
            if seen == active_idx {
                if item.slot_x_anim < 0.0 {
                    item.slot_x_anim = i as f32;
                }
                item.removing = true;
                self.dirty = true;
                return true;
            }
            seen += 1;
        }
        false
    }

    #[allow(dead_code)]
    pub fn remove_clip_at(&mut self, index: usize) -> bool {
        if index >= self.clipboard_items.len() {
            return false;
        }
        self.clipboard_items.remove(index);
        if let Some((ci, _)) = self.copied_feedback {
            if ci == index {
                self.copied_feedback = None;
            } else if ci > index {
                self.copied_feedback = self.copied_feedback.map(|(i, t)| (i - 1, t));
            }
        }
        self.clamp_scrolls();
        self.dirty = true;
        true
    }

    pub fn clamp_scrolls(&mut self) {
        let max_files = self.items.len().saturating_sub(5);
        if self.files_scroll_offset > max_files {
            self.files_scroll_offset = max_files;
        }
        let max_clip = self.clipboard_items.len().saturating_sub(4);
        if self.clip_scroll_offset > max_clip {
            self.clip_scroll_offset = max_clip;
        }
    }

    pub fn scroll_files_by(&mut self, delta: i32) -> bool {
        let max_off = self.items.len().saturating_sub(5);
        let next = (self.files_scroll_offset as i32 + delta).clamp(0, max_off as i32) as usize;
        if next != self.files_scroll_offset {
            self.files_scroll_offset = next;
            true
        } else {
            false
        }
    }

    pub fn scroll_clip_by(&mut self, delta: i32) -> bool {
        let max_off = self.clipboard_items.len().saturating_sub(4);
        let next = (self.clip_scroll_offset as i32 + delta).clamp(0, max_off as i32) as usize;
        if next != self.clip_scroll_offset {
            self.clip_scroll_offset = next;
            true
        } else {
            false
        }
    }

    pub fn step_scroll_animations(&mut self, dt: f32) -> bool {
        self.clamp_scrolls();
        let dt = dt.clamp(0.0, 0.1);
        let rate = 1.0 - (-14.0 * dt).exp();
        let enter_rate = 1.0 - (-13.0 * dt).exp();
        let mut active = false;

        let mut clip_slot = 0usize;
        for item in &mut self.clipboard_items {
            if item.removing {
                item.remove_anim = (item.remove_anim + dt * 5.8).min(1.0);
                active = true;
                continue;
            }
            let target_x = clip_slot as f32;
            clip_slot += 1;
            let dx = target_x - item.slot_x_anim;
            if dx.abs() <= 0.003 {
                item.slot_x_anim = target_x;
            } else {
                item.slot_x_anim += dx * rate;
                active = true;
            }
            if item.enter_anim < 0.999 {
                item.enter_anim += (1.0 - item.enter_anim) * enter_rate;
                if item.enter_anim >= 0.996 {
                    item.enter_anim = 1.0;
                } else {
                    active = true;
                }
            }
        }
        let prev_clip_len = self.clipboard_items.len();
        self.clipboard_items
            .retain(|it| !(it.removing && it.remove_anim >= 0.99));
        if self.clipboard_items.len() != prev_clip_len {
            self.copied_feedback = None;
            self.clamp_scrolls();
            self.dirty = true;
        }

        let mut file_slot = 0usize;
        for item in &mut self.items {
            if item.removing {
                item.remove_anim = (item.remove_anim + dt * 5.8).min(1.0);
                active = true;
                continue;
            }
            let target_x = file_slot as f32;
            file_slot += 1;
            if item.slot_x_anim < 0.0 {
                item.slot_x_anim = target_x;
            } else {
                let dx = target_x - item.slot_x_anim;
                if dx.abs() <= 0.003 {
                    item.slot_x_anim = target_x;
                } else {
                    item.slot_x_anim += dx * rate;
                    active = true;
                }
            }
        }
        let mut removed_any_file = false;
        self.items.retain(|it| {
            let prune = it.removing && it.remove_anim >= 0.99;
            if prune {
                removed_any_file = true;
            }
            !prune
        });
        if removed_any_file {
            self.selected_index = None;
            self.selected_indices.clear();
            self.clamp_scrolls();
            self.dirty = true;
            let _ = self.save();
        }

        let f_target = self.files_scroll_offset as f32;
        let f_diff = f_target - self.files_scroll_anim;
        if f_diff.abs() <= 0.003 {
            self.files_scroll_anim = f_target;
        } else {
            self.files_scroll_anim += f_diff * rate;
            active = true;
        }

        let c_target = self.clip_scroll_offset as f32;
        let c_diff = c_target - self.clip_scroll_anim;
        if c_diff.abs() <= 0.003 {
            self.clip_scroll_anim = c_target;
        } else {
            self.clip_scroll_anim += c_diff * rate;
            active = true;
        }

        active
    }

    pub fn copy_clip_at(&mut self, index: usize) -> bool {
        let Some(text) = self.clipboard_items.get(index).map(|it| it.text.clone()) else {
            return false;
        };
        if !copy_path_to_clipboard(&text) {
            return false;
        }
        self.last_clipboard_seq = unsafe { GetClipboardSequenceNumber() };
        self.copied_feedback = Some((index, Instant::now()));
        self.dirty = true;
        true
    }

    pub fn active_copied_index(&self) -> Option<usize> {
        self.copied_feedback
            .and_then(|(idx, at)| (at.elapsed().as_secs_f32() < 1.4).then_some(idx))
    }

    pub fn copied_flash_t(&self, index: usize) -> f32 {
        let Some((idx, at)) = self.copied_feedback else {
            return 0.0;
        };
        if idx != index {
            return 0.0;
        }
        let elapsed = at.elapsed().as_secs_f32();
        if elapsed < 0.0 || elapsed >= 1.4 {
            return 0.0;
        }
        if elapsed < 0.24 {
            let t = elapsed / 0.24;
            t * t * (3.0 - 2.0 * t)
        } else if elapsed > 0.98 {
            let t = ((1.4 - elapsed) / 0.42).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        } else {
            1.0
        }
    }

    pub fn clear_expired_copied_feedback(&mut self) -> bool {
        if let Some((_, at)) = self.copied_feedback {
            if at.elapsed().as_secs_f32() >= 1.4 {
                self.copied_feedback = None;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    pub fn clear_active_tab(&mut self) {
        match self.active_tab {
            ShelfTab::Files => self.clear(),
            ShelfTab::Clipboard => {
                if !self.clipboard_items.is_empty() || self.copied_feedback.is_some() {
                    self.clipboard_items.clear();
                    self.copied_feedback = None;
                    self.dirty = true;
                }
            }
        }
    }
}

fn clean_path_str(raw: &str) -> String {
    let trimmed = raw.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);
    sanitize_single_line(unquoted)
}

fn sanitize_single_line(s: &str) -> String {
    s.chars()
        .filter(|&c| c != '\r' && c != '\n' && c != '\t')
        .collect()
}

fn contains_path(items: &[ShelfItem], candidate: &str) -> bool {
    let lower = candidate.to_ascii_lowercase();
    items
        .iter()
        .any(|it| it.path.to_ascii_lowercase() == lower)
}

struct IconGuard(HICON);

impl Drop for IconGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

struct IconInfoGuard(ICONINFO);

impl Drop for IconInfoGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.0.hbmColor.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.0.hbmColor.0));
            }
            if !self.0.hbmMask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.0.hbmMask.0));
            }
        }
    }
}

struct DcGuard(HDC);

impl Drop for DcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
}

static SHELL_ICON_LOCK: Mutex<()> = Mutex::new(());

/// and converts it to premultiplied BGRA8 pixels.
pub fn load_windows_shell_icon(path: &str, is_dir: bool) -> Option<(u32, u32, Vec<u8>)> {
    if path.trim().is_empty() {
        return None;
    }
    let normalized = path.replace('/', "\\");
    let wide: Vec<u16> = normalized.encode_utf16().chain(std::iter::once(0)).collect();
    let attrs: FILE_FLAGS_AND_ATTRIBUTES = if is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };

    let _guard = SHELL_ICON_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ole = init_ole().ok();
    let hicon = obtain_shell_hicon(&wide, attrs)?;
    let _icon_guard = IconGuard(hicon);
    hicon_to_premultiplied_bgra(hicon)
}

fn obtain_shell_hicon(wide: &[u16], attrs: FILE_FLAGS_AND_ATTRIBUTES) -> Option<HICON> {
    let mut sfi = SHFILEINFOW::default();
    let sfi_size = std::mem::size_of::<SHFILEINFOW>() as u32;

    let idx_flags: SHGFI_FLAGS = SHGFI_SYSICONINDEX | SHGFI_LARGEICON;
    let mut ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut sfi),
            sfi_size,
            idx_flags,
        )
    } != 0;
    if !ok {
        ok = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                attrs,
                Some(&mut sfi),
                sfi_size,
                idx_flags | SHGFI_USEFILEATTRIBUTES,
            )
        } != 0;
    }
    if ok {
        if let Ok(ilist) = unsafe { SHGetImageList::<IImageList>(SHIL_EXTRALARGE as i32) } {
            if let Ok(icon) = unsafe { ilist.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0) } {
                if !icon.is_invalid() {
                    return Some(icon);
                }
            }
        }
    }

    sfi = SHFILEINFOW::default();
    let direct_flags: SHGFI_FLAGS = SHGFI_ICON | SHGFI_LARGEICON;
    let mut got = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut sfi),
            sfi_size,
            direct_flags,
        )
    } != 0;
    if !got || sfi.hIcon.is_invalid() {
        sfi = SHFILEINFOW::default();
        got = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                attrs,
                Some(&mut sfi),
                sfi_size,
                direct_flags | SHGFI_USEFILEATTRIBUTES,
            )
        } != 0;
    }
    if got && !sfi.hIcon.is_invalid() {
        Some(sfi.hIcon)
    } else {
        None
    }
}

fn hicon_to_premultiplied_bgra(hicon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = ICONINFO::default();
    unsafe {
        if GetIconInfo(hicon, &mut info).is_err() {
            return None;
        }
    }
    let info_guard = IconInfoGuard(info);
    let hbm_color: HBITMAP = info_guard.0.hbmColor;
    let hbm_mask: HBITMAP = info_guard.0.hbmMask;
    if hbm_color.is_invalid() {
        return None;
    }

    let mut bmp = BITMAP::default();
    let got_bmp = unsafe {
        GetObjectW(
            HGDIOBJ(hbm_color.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut BITMAP as *mut _),
        )
    };
    if got_bmp == 0 {
        return None;
    }
    let width = bmp.bmWidth;
    let height = bmp.bmHeight.abs();
    if width <= 0 || height <= 0 || width > 512 || height > 512 {
        return None;
    }

    let hdc = unsafe { CreateCompatibleDC(HDC::default()) };
    if hdc.is_invalid() {
        return None;
    }
    let _dc_guard = DcGuard(hdc);

    let pixel_count = (width as usize) * (height as usize);
    let mut bgra = vec![0u8; pixel_count * 4];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let scanlines = unsafe {
        GetDIBits(
            hdc,
            hbm_color,
            0,
            height as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    if scanlines == 0 {
        return None;
    }

    let has_alpha = bgra.chunks_exact(4).any(|px| px[3] != 0);
    if !has_alpha {
        if !hbm_mask.is_invalid() {
            let mut mask_bmp = BITMAP::default();
            let got_mask = unsafe {
                GetObjectW(
                    HGDIOBJ(hbm_mask.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some(&mut mask_bmp as *mut BITMAP as *mut _),
                )
            };
            let mask_h = if got_mask != 0 {
                mask_bmp.bmHeight.abs().max(height).min(1024)
            } else {
                height
            };
            let mut mask_buf = vec![0u8; (width as usize) * (mask_h as usize) * 4];
            let mut mask_bmi = bmi;
            mask_bmi.bmiHeader.biHeight = -mask_h;
            let mask_lines = unsafe {
                GetDIBits(
                    hdc,
                    hbm_mask,
                    0,
                    mask_h as u32,
                    Some(mask_buf.as_mut_ptr() as *mut _),
                    &mut mask_bmi,
                    DIB_RGB_COLORS,
                )
            };
            if mask_lines > 0 {
                for (px, mask_px) in bgra
                    .chunks_exact_mut(4)
                    .zip(mask_buf[..pixel_count * 4].chunks_exact(4))
                {
                    px[3] = if mask_px[0] == 0 { 255 } else { 0 };
                }
            } else {
                for px in bgra.chunks_exact_mut(4) {
                    px[3] = 255;
                }
            }
        } else {
            for px in bgra.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
    }

    for px in bgra.chunks_exact_mut(4) {
        let a = px[3] as u32;
        px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
    }

    Some((width as u32, height as u32, bgra))
}

pub fn open_path(path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let hinst = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    (hinst.0 as isize) > 32
}

#[allow(dead_code)]
struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn read_clipboard_unicode_text() -> Option<String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let _guard = ClipboardGuard;
        let Ok(handle) = GetClipboardData(CF_UNICODETEXT.0 as u32) else {
            return None;
        };
        let hmem = HGLOBAL(handle.0);
        if hmem.is_invalid() {
            return None;
        }
        let ptr = GlobalLock(hmem) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let byte_size = GlobalSize(hmem);
        let max_u16 = byte_size / std::mem::size_of::<u16>();
        let result = if max_u16 == 0 {
            None
        } else {
            let slice = std::slice::from_raw_parts(ptr, max_u16);
            let len = slice.iter().position(|&c| c == 0).unwrap_or(max_u16);
            Some(String::from_utf16_lossy(&slice[..len]))
        };
        let _ = GlobalUnlock(hmem);
        result
    }
}

pub fn copy_path_to_clipboard(path: &str) -> bool {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _guard = ClipboardGuard;

        if EmptyClipboard().is_err() {
            return false;
        }

        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, byte_len) else {
            return false;
        };

        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(hmem);
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(hmem);

        if SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(hmem.0)).is_err() {
            let _ = GlobalFree(hmem);
            return false;
        }
        true
    }
}

pub struct OleGuard;

impl Drop for OleGuard {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

/// Initializes OLE on the current STA UI thread (required for `RegisterDragDrop` / `DoDragDrop`).
pub fn init_ole() -> windows::core::Result<OleGuard> {
    unsafe {
        OleInitialize(None)?;
    }
    Ok(OleGuard)
}

pub struct DropTargetRegistration {
    hwnd: HWND,
}

impl Drop for DropTargetRegistration {
    fn drop(&mut self) {
        unsafe {
            let _ = RevokeDragDrop(self.hwnd);
        }
    }
}

/// Registers an OLE `IDropTarget` on `hwnd` wired directly to `store`.
pub fn register_drop_target(
    hwnd: HWND,
    store: Rc<RefCell<ShelfStore>>,
) -> windows::core::Result<DropTargetRegistration> {
    let target: IDropTarget = ShelfDropTarget::new(hwnd, store).into();
    unsafe {
        RegisterDragDrop(hwnd, &target)?;
    }
    Ok(DropTargetRegistration { hwnd })
}

#[implement(IDropTarget)]
struct ShelfDropTarget {
    hwnd: HWND,
    store: Rc<RefCell<ShelfStore>>,
    has_files: Cell<bool>,
}

impl ShelfDropTarget {
    fn new(hwnd: HWND, store: Rc<RefCell<ShelfStore>>) -> Self {
        Self {
            hwnd,
            store,
            has_files: Cell::new(false),
        }
    }

    fn wake_window(&self) {
        unsafe {
            let _ = PostMessageW(self.hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn has_hdrop_data(pdataobj: Option<&IDataObject>) -> bool {
    let Some(obj) = pdataobj else {
        return false;
    };
    let fmt = hdrop_format();
    unsafe { obj.QueryGetData(&fmt) == S_OK }
}

struct StgMediumGuard(STGMEDIUM);

impl Drop for StgMediumGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseStgMedium(&mut self.0);
        }
    }
}

fn extract_hdrop_paths(pdataobj: Option<&IDataObject>) -> Vec<String> {
    let Some(obj) = pdataobj else {
        return Vec::new();
    };
    let fmt = hdrop_format();
    let Ok(medium) = (unsafe { obj.GetData(&fmt) }) else {
        return Vec::new();
    };
    let guard = StgMediumGuard(medium);
    if guard.0.tymed != TYMED_HGLOBAL.0 as u32 {
        return Vec::new();
    }
    let hdrop = HDROP(unsafe { guard.0.u.hGlobal.0 });
    if hdrop.is_invalid() {
        return Vec::new();
    }

    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for idx in 0..count {
        let len = unsafe { DragQueryFileW(hdrop, idx, None) };
        if len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let written = unsafe { DragQueryFileW(hdrop, idx, Some(&mut buf)) };
        if written > 0 {
            paths.push(String::from_utf16_lossy(&buf[..written as usize]));
        }
    }
    paths
}

thread_local! {
    static DRAG_OUT_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

struct DragOutGuard;

impl DragOutGuard {
    fn enter() -> Self {
        DRAG_OUT_ACTIVE.with(|f| f.set(true));
        Self
    }
}

impl Drop for DragOutGuard {
    fn drop(&mut self) {
        DRAG_OUT_ACTIVE.with(|f| f.set(false));
    }
}

fn is_drag_out_active() -> bool {
    DRAG_OUT_ACTIVE.with(|f| f.get())
}

impl IDropTarget_Impl for ShelfDropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Option<&IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let valid = !is_drag_out_active() && has_hdrop_data(pdataobj);
        self.has_files.set(valid);
        {
            let mut store = self.store.borrow_mut();
            let next_pt = valid.then_some((pt.x, pt.y));
            if store.drag_cursor_screen_px != next_pt {
                store.drag_cursor_screen_px = next_pt;
                store.dirty = true;
            }
            store.set_drag_over(valid);
        }
        if !pdweffect.is_null() {
            unsafe {
                *pdweffect = if valid { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
            }
        }
        self.wake_window();
        Ok(())
    }

    fn DragOver(
        &self,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let valid = !is_drag_out_active() && self.has_files.get();
        if valid {
            let mut store = self.store.borrow_mut();
            let next_pt = Some((pt.x, pt.y));
            if store.drag_cursor_screen_px != next_pt {
                store.drag_cursor_screen_px = next_pt;
                store.dirty = true;
                self.wake_window();
            }
        }
        if !pdweffect.is_null() {
            unsafe {
                *pdweffect = if valid {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.has_files.set(false);
        {
            let mut store = self.store.borrow_mut();
            if store.drag_cursor_screen_px.take().is_some() {
                store.dirty = true;
            }
            store.set_drag_over(false);
        }
        self.wake_window();
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Option<&IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.has_files.set(false);
        if is_drag_out_active() {
            let mut store = self.store.borrow_mut();
            store.drag_cursor_screen_px = None;
            store.set_drag_over(false);
            if !pdweffect.is_null() {
                unsafe {
                    *pdweffect = DROPEFFECT_NONE;
                }
            }
            return Ok(());
        }
        let paths = extract_hdrop_paths(pdataobj);
        {
            let mut store = self.store.borrow_mut();
            store.drag_cursor_screen_px = None;
            store.pending_drop_screen_px = Some((pt.x, pt.y));
            store.set_drag_over(false);
            store.add_paths(&paths);
            store.dirty = true;
        }
        if !pdweffect.is_null() {
            unsafe {
                *pdweffect = if paths.is_empty() {
                    DROPEFFECT_NONE
                } else {
                    DROPEFFECT_COPY
                };
            }
        }
        self.wake_window();
        Ok(())
    }
}

#[implement(IDropSource)]
struct ShelfDropSource;

impl IDropSource_Impl for ShelfDropSource_Impl {
    fn QueryContinueDrag(
        &self,
        fescapepressed: BOOL,
        grfkeystate: MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        if fescapepressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if (grfkeystate & MK_LBUTTON) == MODIFIERKEYS_FLAGS(0) {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

struct PidlList(Vec<*mut ITEMIDLIST>);

impl Drop for PidlList {
    fn drop(&mut self) {
        for pidl in self.0.drain(..) {
            if !pidl.is_null() {
                unsafe {
                    ILFree(Some(pidl as *const ITEMIDLIST));
                }
            }
        }
    }
}

pub fn start_drag_out(hwnd: HWND, paths: &[String]) -> windows::core::Result<bool> {
    let _ = hwnd;
    let mut pidls = PidlList(Vec::new());
    for raw in paths {
        let cleaned = clean_path_str(raw);
        if cleaned.is_empty() {
            continue;
        }
        let hpath = HSTRING::from(cleaned.as_str());
        if let Ok(item) = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&hpath, None) } {
            if let Ok(pidl) = unsafe { SHGetIDListFromObject(&item) } {
                if !pidl.is_null() {
                    pidls.0.push(pidl);
                }
            }
        }
    }

    if pidls.0.is_empty() {
        return Ok(false);
    }

    let const_pidls: Vec<*const ITEMIDLIST> =
        pidls.0.iter().map(|&p| p as *const ITEMIDLIST).collect();
    let array = unsafe { SHCreateShellItemArrayFromIDLists(&const_pidls)? };
    let data_obj: IDataObject = unsafe { array.BindToHandler(None, &BHID_DataObject)? };
    let drop_source: IDropSource = ShelfDropSource.into();

    let _drag_guard = DragOutGuard::enter();
    let mut effect = DROPEFFECT(0);
    let hr = unsafe {
        DoDragDrop(
            &data_obj,
            &drop_source,
            DROPEFFECT_COPY,
            &mut effect,
        )
    };

    if hr == DRAGDROP_S_DROP {
        Ok(true)
    } else if hr == DRAGDROP_S_CANCEL {
        Ok(false)
    } else {
        hr.ok().map(|()| false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_store_parsing_and_roundtrip() {
        let unique = std::process::id();
        let path = env::temp_dir().join(format!("eave_shelf_test_{unique}.txt"));
        let _ = fs::remove_file(&path);

        let raw = "\
# Eave shelf
F\t2048\tC:\\Temp\\notes.txt
D\t0\tC:\\Temp\\folder
C:\\Temp\\plain_entry.pdf
# duplicate case-insensitive path should be skipped
f\t999\tC:\\temp\\NOTES.TXT
";
        let parsed = ShelfStore::parse_str(raw);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name, "notes.txt");
        assert!(!parsed[0].is_dir);
        assert_eq!(parsed[0].size_bytes, 2048);
        assert_eq!(parsed[1].name, "folder");
        assert!(parsed[1].is_dir);
        assert_eq!(parsed[2].name, "plain_entry.pdf");

        let mut store = ShelfStore::load_from(&path);
        assert!(store.items.is_empty());

        let added = store.add_paths(&[
            "\"C:\\Work\\design.fig\"".to_string(),
            "C:\\work\\DESIGN.FIG".to_string(),
            "   ".to_string(),
            "C:\\Work\\report.pdf".to_string(),
        ]);
        assert_eq!(added, 2);
        assert!(store.take_dirty());
        assert!(!store.take_dirty());

        let reloaded = ShelfStore::load_from(&path);
        assert_eq!(reloaded.items.len(), 2);
        assert_eq!(reloaded.items[0].path, "C:\\Work\\design.fig");
        assert_eq!(reloaded.items[0].extension_label(), "FIG");
        assert_eq!(reloaded.items[1].name, "report.pdf");

        assert!(store.remove_at(0).is_some());
        assert_eq!(store.items.len(), 1);
        store.clear();
        assert!(store.items.is_empty());

        let reloaded_empty = ShelfStore::load_from(&path);
        assert!(reloaded_empty.items.is_empty());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn shelf_item_size_formatting() {
        let make_file = |size_bytes: u64| ShelfItem {
            path: "a.bin".into(),
            name: "a.bin".into(),
            is_dir: false,
            size_bytes,
            icon_width: 0,
            icon_height: 0,
            icon_bgra: None,
            slot_x_anim: -1.0,
            removing: false,
            remove_anim: 0.0,
        };
        assert_eq!(make_file(512).format_size(), "512 B");
        assert_eq!(make_file(1536).format_size(), "1.5 KB");
        assert_eq!(make_file(830 * 1024).format_size(), "830 KB");
        assert_eq!(make_file(2_516_582).format_size(), "2.4 MB");
        assert_eq!(make_file(15 * 1024 * 1024).format_size(), "15 MB");

        let dir = ShelfItem {
            path: "docs".into(),
            name: "docs".into(),
            is_dir: true,
            size_bytes: 0,
            icon_width: 0,
            icon_height: 0,
            icon_bgra: None,
            slot_x_anim: -1.0,
            removing: false,
            remove_anim: 0.0,
        };
        assert_eq!(dir.format_size(), "Folder");
        assert_eq!(dir.extension_label(), "DIR");
    }

    #[test]
    fn shelf_clipboard_tab_dedup_feedback_and_clear() {
        let unique = std::process::id();
        let path = env::temp_dir().join(format!("eave_shelf_clip_test_{unique}.txt"));
        let _ = fs::remove_file(&path);

        let mut store = ShelfStore::load_from(&path);
        assert_eq!(store.active_tab, ShelfTab::Files);
        assert!(store.clipboard_items.is_empty());

        store.seed_demo_clipboard();
        assert_eq!(store.clipboard_items.len(), 4);
        assert!(store.clipboard_items[0].is_new);
        assert_eq!(store.clipboard_items[0].time_label, "now");

        assert!(!store.push_clipboard_text("   "));
        assert!(!store.push_clipboard_text("https://eave.app/releases/v0.1"));

        assert!(store.push_clipboard_text("hello world"));
        assert!(store.take_dirty());
        assert_eq!(store.clipboard_items[0].text, "hello world");
        assert_eq!(store.clipboard_items[0].time_label, "now");
        assert!(store.clipboard_items[0].is_new);
        assert!(!store.clipboard_items[1].is_new);

        for i in 0..26 {
            assert!(store.push_clipboard_text(&format!("clip-{i}")));
        }
        assert_eq!(store.clipboard_items.len(), 24);
        assert_eq!(store.clipboard_items[0].text, "clip-25");

        assert!(store.scroll_clip_by(3));
        assert_eq!(store.clip_scroll_offset, 3);
        assert!(store.scroll_clip_by(100));
        assert_eq!(store.clip_scroll_offset, 20);
        assert!(store.remove_clip_at(0));
        assert_eq!(store.clipboard_items.len(), 23);
        assert_eq!(store.clip_scroll_offset, 19);
        assert!(store.scroll_clip_by(-100));
        assert_eq!(store.clip_scroll_offset, 0);

        store.copied_feedback = Some((1, Instant::now()));
        assert_eq!(store.active_copied_index(), Some(1));
        assert!(!store.clear_expired_copied_feedback());

        store.copied_feedback = Some((2, Instant::now() - std::time::Duration::from_secs(2)));
        assert_eq!(store.active_copied_index(), None);
        assert!(store.clear_expired_copied_feedback());
        assert!(store.copied_feedback.is_none());

        store.pending_drop_screen_px = Some((640, 80));
        assert_eq!(store.take_drop_screen_px(), Some((640, 80)));
        assert_eq!(store.take_drop_screen_px(), None);

        store.add_paths(&["C:\\Temp\\keep_me.txt".to_string()]);
        store.active_tab = ShelfTab::Clipboard;
        store.clear_active_tab();
        assert!(store.clipboard_items.is_empty());
        assert_eq!(store.items.len(), 1);

        store.active_tab = ShelfTab::Files;
        store.clear_active_tab();
        assert!(store.items.is_empty());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn shelf_multi_select_and_shell_icon_extraction() {
        let unique = std::process::id();
        let path = env::temp_dir().join(format!("eave_shelf_sel_test_{unique}.txt"));
        let _ = fs::remove_file(&path);

        let _ole = init_ole().ok();
        let mut store = ShelfStore::load_from(&path);
        store.add_paths(&[
            "C:\\Windows\\System32\\notepad.exe".to_string(),
            "C:\\Temp\\doc.txt".to_string(),
            "C:\\Temp\\folder".to_string(),
        ]);

        // Windows Shell icon extraction returns valid premultiplied BGRA8 pixels.
        if let Some(bgra) = &store.items[0].icon_bgra {
            assert!(store.items[0].icon_width > 0);
            assert!(store.items[0].icon_height > 0);
            assert_eq!(
                bgra.len(),
                (store.items[0].icon_width * store.items[0].icon_height * 4) as usize
            );
            for px in bgra.chunks_exact(4) {
                assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3]);
            }
        }

        assert!(!store.select_mode);
        assert_eq!(
            store.drag_paths_for_item(1),
            vec!["C:\\Temp\\doc.txt".to_string()]
        );

        store.toggle_select_mode();
        assert!(store.select_mode);
        store.toggle_item_selected(0);
        store.toggle_item_selected(2);
        assert!(store.is_item_selected(0));
        assert!(!store.is_item_selected(1));
        assert!(store.is_item_selected(2));

        assert_eq!(
            store.drag_paths_for_item(1),
            vec!["C:\\Temp\\doc.txt".to_string()]
        );
        assert_eq!(
            store.drag_paths_for_item(2),
            vec![
                "C:\\Windows\\System32\\notepad.exe".to_string(),
                "C:\\Temp\\folder".to_string(),
            ]
        );

        assert!(store.remove_at(0).is_some());
        assert_eq!(store.selected_indices, vec![1]);

        store.toggle_select_mode();
        assert!(!store.select_mode);
        assert!(store.selected_indices.is_empty());

        let _ = fs::remove_file(&path);
    }
}
