//! Window / tab / split layout. The Rust side owns it; windows only render it.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::paths::atomic_write;
use crate::session::{ConnectSpec, SessionId};

pub type PaneId = u64;
pub type TabId = u64;
pub type SplitId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Side by side (split to the right).
    Horizontal,
    /// Stacked (split down).
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub id: PaneId,
    /// What this panel connects to; `None` for an empty panel.
    pub spec: Option<ConnectSpec>,
    /// Live session. Not persisted.
    #[serde(default, skip_deserializing)]
    pub session: Option<SessionId>,
    /// Project override for this panel (takes precedence over the server's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Restored panel waiting for the user to allow reconnecting. Not persisted.
    #[serde(default, skip_deserializing)]
    pub held: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    Pane(Pane),
    Split {
        id: SplitId,
        direction: Direction,
        /// Share of the first child, 0..1.
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    fn panes<'a>(&'a self, out: &mut Vec<&'a Pane>) {
        match self {
            Node::Pane(p) => out.push(p),
            Node::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        match self {
            Node::Pane(p) if p.id == id => Some(p),
            Node::Pane(_) => None,
            Node::Split { first, second, .. } => first.pane_mut(id).or_else(|| second.pane_mut(id)),
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Pane(p) => p.id == id,
            Node::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Replaces the pane `id` with `f(pane)`.
    fn replace_pane(&mut self, id: PaneId, f: &mut dyn FnMut(Node) -> Node) -> bool {
        match self {
            Node::Pane(p) if p.id == id => {
                let old = std::mem::replace(
                    self,
                    Node::Pane(Pane {
                        id: 0,
                        spec: None,
                        session: None,
                        held: false,
                        project: None,
                    }),
                );
                *self = f(old);
                true
            }
            Node::Pane(_) => false,
            Node::Split { first, second, .. } => {
                first.replace_pane(id, f) || second.replace_pane(id, f)
            }
        }
    }

    /// Removes the pane; its sibling takes the parent split's place.
    /// Returns `None` if this node itself is the pane (caller removes the whole tree).
    fn remove_pane(self, id: PaneId) -> Option<Node> {
        match self {
            Node::Pane(p) if p.id == id => None,
            Node::Pane(_) => Some(self),
            Node::Split {
                id: sid,
                direction,
                ratio,
                first,
                second,
            } => {
                if first.contains(id) {
                    match first.remove_pane(id) {
                        None => Some(*second),
                        Some(f) => Some(Node::Split {
                            id: sid,
                            direction,
                            ratio,
                            first: Box::new(f),
                            second,
                        }),
                    }
                } else if second.contains(id) {
                    match second.remove_pane(id) {
                        None => Some(*first),
                        Some(s) => Some(Node::Split {
                            id: sid,
                            direction,
                            ratio,
                            first,
                            second: Box::new(s),
                        }),
                    }
                } else {
                    Some(Node::Split {
                        id: sid,
                        direction,
                        ratio,
                        first,
                        second,
                    })
                }
            }
        }
    }

    fn set_ratio(&mut self, split: SplitId, value: f64) -> bool {
        match self {
            Node::Pane(_) => false,
            Node::Split {
                id,
                ratio,
                first,
                second,
                ..
            } => {
                if *id == split {
                    *ratio = value.clamp(0.05, 0.95);
                    true
                } else {
                    first.set_ratio(split, value) || second.set_ratio(split, value)
                }
            }
        }
    }

    fn max_id(&self) -> u64 {
        match self {
            Node::Pane(p) => p.id,
            Node::Split {
                id, first, second, ..
            } => (*id).max(first.max_id()).max(second.max_id()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    /// Custom title; otherwise the UI shows the focused panel's server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub root: Node,
    pub focused: PaneId,
}

impl Tab {
    pub fn panes(&self) -> Vec<&Pane> {
        let mut out = Vec::new();
        self.root.panes(&mut out);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub label: String,
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    #[serde(default)]
    pub bounds: Option<Bounds>,
    #[serde(default)]
    pub maximized: bool,
}

impl WindowState {
    pub fn sessions(&self) -> Vec<SessionId> {
        self.tabs
            .iter()
            .flat_map(|t| {
                t.panes()
                    .into_iter()
                    .filter_map(|p| p.session)
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub windows: Vec<WindowState>,
    #[serde(skip)]
    next_id: u64,
}

/// What was removed by a close operation: sessions to close and whether the
/// window became empty.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub sessions: Vec<SessionId>,
    pub window_empty: Option<String>,
}

fn not_found(what: &str) -> CoreError {
    CoreError::new(ErrorKind::NotFound, format!("{what} не найден(а)"))
}

impl Layout {
    fn next(&mut self) -> u64 {
        if self.next_id == 0 {
            self.next_id = self
                .windows
                .iter()
                .flat_map(|w| w.tabs.iter())
                .map(|t| t.id.max(t.root.max_id()))
                .max()
                .unwrap_or(0);
        }
        self.next_id += 1;
        self.next_id
    }

    pub fn window(&self, label: &str) -> Option<&WindowState> {
        self.windows.iter().find(|w| w.label == label)
    }

    fn window_mut(&mut self, label: &str) -> CoreResult<&mut WindowState> {
        self.windows
            .iter_mut()
            .find(|w| w.label == label)
            .ok_or_else(|| not_found("Окно"))
    }

    pub fn add_window(&mut self, label: &str, bounds: Option<Bounds>) {
        if self.window(label).is_none() {
            self.windows.push(WindowState {
                label: label.to_owned(),
                tabs: Vec::new(),
                active: None,
                bounds,
                maximized: false,
            });
        }
    }

    /// Removes the window; returns the sessions of its panels.
    pub fn remove_window(&mut self, label: &str) -> Vec<SessionId> {
        let Some(pos) = self.windows.iter().position(|w| w.label == label) else {
            return Vec::new();
        };
        self.windows.remove(pos).sessions()
    }

    pub fn set_bounds(&mut self, label: &str, bounds: Bounds, maximized: bool) {
        if let Ok(w) = self.window_mut(label) {
            if !maximized {
                w.bounds = Some(bounds);
            }
            w.maximized = maximized;
        }
    }

    /// Locates a pane: (window index, tab index).
    fn find_pane(&self, pane: PaneId) -> Option<(usize, usize)> {
        for (wi, w) in self.windows.iter().enumerate() {
            for (ti, t) in w.tabs.iter().enumerate() {
                if t.root.contains(pane) {
                    return Some((wi, ti));
                }
            }
        }
        None
    }

    fn find_tab(&self, tab: TabId) -> Option<(usize, usize)> {
        for (wi, w) in self.windows.iter().enumerate() {
            if let Some(ti) = w.tabs.iter().position(|t| t.id == tab) {
                return Some((wi, ti));
            }
        }
        None
    }

    pub fn pane(&self, pane: PaneId) -> Option<&Pane> {
        let (wi, ti) = self.find_pane(pane)?;
        self.windows[wi].tabs[ti]
            .panes()
            .into_iter()
            .find(|p| p.id == pane)
    }

    pub fn pane_window(&self, pane: PaneId) -> Option<&str> {
        self.find_pane(pane)
            .map(|(wi, _)| self.windows[wi].label.as_str())
    }

    pub fn tab_window(&self, tab: TabId) -> Option<&str> {
        self.find_tab(tab)
            .map(|(wi, _)| self.windows[wi].label.as_str())
    }

    pub fn tab(&self, tab: TabId) -> Option<&Tab> {
        self.find_tab(tab)
            .map(|(wi, ti)| &self.windows[wi].tabs[ti])
    }

    fn pane_mut(&mut self, pane: PaneId) -> CoreResult<&mut Pane> {
        let (wi, ti) = self.find_pane(pane).ok_or_else(|| not_found("Панель"))?;
        self.windows[wi].tabs[ti]
            .root
            .pane_mut(pane)
            .ok_or_else(|| not_found("Панель"))
    }

    /// Opens a new tab with one panel at the end of the window (or at `index`).
    pub fn new_tab(
        &mut self,
        window: &str,
        spec: Option<ConnectSpec>,
        index: Option<usize>,
    ) -> CoreResult<(TabId, PaneId)> {
        let tab_id = self.next();
        let pane_id = self.next();
        let w = self.window_mut(window)?;
        let tab = Tab {
            id: tab_id,
            title: None,
            color: None,
            root: Node::Pane(Pane {
                id: pane_id,
                spec,
                session: None,
                held: false,
                project: None,
            }),
            focused: pane_id,
        };
        let at = index.unwrap_or(w.tabs.len()).min(w.tabs.len());
        w.tabs.insert(at, tab);
        w.active = Some(tab_id);
        Ok((tab_id, pane_id))
    }

    /// Splits the pane; the new panel goes right (horizontal) or below (vertical).
    pub fn split(
        &mut self,
        pane: PaneId,
        direction: Direction,
        spec: Option<ConnectSpec>,
    ) -> CoreResult<PaneId> {
        let (wi, ti) = self.find_pane(pane).ok_or_else(|| not_found("Панель"))?;
        let split_id = self.next();
        let new_id = self.next();
        let tab = &mut self.windows[wi].tabs[ti];
        let mut spec = Some(spec);
        tab.root.replace_pane(pane, &mut |old| Node::Split {
            id: split_id,
            direction,
            ratio: 0.5,
            first: Box::new(old),
            second: Box::new(Node::Pane(Pane {
                id: new_id,
                spec: spec.take().flatten(),
                session: None,
                held: false,
                project: None,
            })),
        });
        tab.focused = new_id;
        Ok(new_id)
    }

    pub fn set_session(&mut self, pane: PaneId, session: Option<SessionId>) -> CoreResult<()> {
        self.pane_mut(pane)?.session = session;
        Ok(())
    }

    pub fn set_pane_project(&mut self, pane: PaneId, project: Option<String>) -> CoreResult<()> {
        self.pane_mut(pane)?.project = crate::project::normalize(project);
        Ok(())
    }

    pub fn set_spec(&mut self, pane: PaneId, spec: Option<ConnectSpec>) -> CoreResult<()> {
        self.pane_mut(pane)?.spec = spec;
        Ok(())
    }

    pub fn set_ratio(&mut self, tab: TabId, split: SplitId, ratio: f64) -> CoreResult<()> {
        let (wi, ti) = self.find_tab(tab).ok_or_else(|| not_found("Вкладка"))?;
        if self.windows[wi].tabs[ti].root.set_ratio(split, ratio) {
            Ok(())
        } else {
            Err(not_found("Разделитель"))
        }
    }

    pub fn focus_pane(&mut self, pane: PaneId) -> CoreResult<()> {
        let (wi, ti) = self.find_pane(pane).ok_or_else(|| not_found("Панель"))?;
        let w = &mut self.windows[wi];
        w.tabs[ti].focused = pane;
        w.active = Some(w.tabs[ti].id);
        Ok(())
    }

    pub fn activate_tab(&mut self, tab: TabId) -> CoreResult<()> {
        let (wi, _) = self.find_tab(tab).ok_or_else(|| not_found("Вкладка"))?;
        self.windows[wi].active = Some(tab);
        Ok(())
    }

    pub fn rename_tab(&mut self, tab: TabId, title: Option<String>) -> CoreResult<()> {
        let (wi, ti) = self.find_tab(tab).ok_or_else(|| not_found("Вкладка"))?;
        self.windows[wi].tabs[ti].title =
            title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        Ok(())
    }

    pub fn set_tab_color(&mut self, tab: TabId, color: Option<String>) -> CoreResult<()> {
        let (wi, ti) = self.find_tab(tab).ok_or_else(|| not_found("Вкладка"))?;
        self.windows[wi].tabs[ti].color = color.filter(|c| !c.is_empty());
        Ok(())
    }

    /// Removes a pane (and its tab if it was the last one).
    pub fn close_pane(&mut self, pane: PaneId) -> CoreResult<Removed> {
        let (wi, ti) = self.find_pane(pane).ok_or_else(|| not_found("Панель"))?;
        let session = self.pane(pane).and_then(|p| p.session);
        let w = &mut self.windows[wi];
        let tab = &mut w.tabs[ti];
        let root = std::mem::replace(
            &mut tab.root,
            Node::Pane(Pane {
                id: 0,
                spec: None,
                session: None,
                held: false,
                project: None,
            }),
        );
        let mut removed = Removed {
            sessions: session.into_iter().collect(),
            window_empty: None,
        };
        match root.remove_pane(pane) {
            Some(rest) => {
                tab.root = rest;
                if tab.focused == pane {
                    let mut panes = Vec::new();
                    tab.root.panes(&mut panes);
                    tab.focused = panes.first().map(|p| p.id).unwrap_or(0);
                }
            }
            None => {
                let tab_id = tab.id;
                Self::drop_tab(w, ti, tab_id);
                if w.tabs.is_empty() {
                    removed.window_empty = Some(w.label.clone());
                }
            }
        }
        Ok(removed)
    }

    fn drop_tab(w: &mut WindowState, ti: usize, tab_id: TabId) {
        w.tabs.remove(ti);
        if w.active == Some(tab_id) {
            w.active = w
                .tabs
                .get(ti.min(w.tabs.len().saturating_sub(1)))
                .map(|t| t.id);
        }
    }

    pub fn close_tab(&mut self, tab: TabId) -> CoreResult<Removed> {
        let (wi, ti) = self.find_tab(tab).ok_or_else(|| not_found("Вкладка"))?;
        let w = &mut self.windows[wi];
        let sessions = w.tabs[ti]
            .panes()
            .iter()
            .filter_map(|p| p.session)
            .collect();
        Self::drop_tab(w, ti, tab);
        let window_empty = w.tabs.is_empty().then(|| w.label.clone());
        Ok(Removed {
            sessions,
            window_empty,
        })
    }

    /// Moves a tab to `window` at `index` (end if `None`); also used for reordering.
    /// Sessions are untouched, so nothing reconnects. Returns the source window if it
    /// became empty.
    pub fn move_tab(
        &mut self,
        tab: TabId,
        window: &str,
        index: Option<usize>,
    ) -> CoreResult<Option<String>> {
        let (wi, ti) = self.find_tab(tab).ok_or_else(|| not_found("Вкладка"))?;
        if self.window(window).is_none() {
            return Err(not_found("Окно"));
        }
        let src = &mut self.windows[wi];
        let moved = src.tabs.remove(ti);
        let src_label = src.label.clone();
        if src.active == Some(tab) {
            src.active = src
                .tabs
                .get(ti.min(src.tabs.len().saturating_sub(1)))
                .map(|t| t.id);
        }
        let src_empty = src.tabs.is_empty() && src_label != window;

        let dst = self.window_mut(window)?;
        let mut at = index.unwrap_or(dst.tabs.len());
        if src_label == window && index.is_some_and(|i| i > ti) {
            at -= 1; // the tab was removed before the insertion point
        }
        at = at.min(dst.tabs.len());
        dst.tabs.insert(at, moved);
        dst.active = Some(tab);
        Ok(src_empty.then_some(src_label))
    }

    /// Marks every panel with a connection spec as waiting for permission to connect.
    pub fn hold_all(&mut self) {
        for w in &mut self.windows {
            for t in &mut w.tabs {
                for_each_pane(&mut t.root, &mut |p| p.held = p.spec.is_some());
            }
        }
    }

    /// Allows held panels to connect. Returns `true` if anything changed.
    pub fn release_all(&mut self) -> bool {
        let mut changed = false;
        for w in &mut self.windows {
            for t in &mut w.tabs {
                for_each_pane(&mut t.root, &mut |p| {
                    changed |= p.held;
                    p.held = false;
                });
            }
        }
        changed
    }

    pub fn release(&mut self, pane: PaneId) -> CoreResult<()> {
        self.pane_mut(pane)?.held = false;
        Ok(())
    }

    pub fn all_sessions(&self) -> Vec<SessionId> {
        self.windows.iter().flat_map(|w| w.sessions()).collect()
    }

    /// Copy for saving: no live session ids.
    pub fn persistable(&self) -> Layout {
        let mut copy = self.clone();
        for w in &mut copy.windows {
            for t in &mut w.tabs {
                clear_sessions(&mut t.root);
            }
        }
        copy
    }
}

fn for_each_pane(node: &mut Node, f: &mut dyn FnMut(&mut Pane)) {
    match node {
        Node::Pane(p) => f(p),
        Node::Split { first, second, .. } => {
            for_each_pane(first, f);
            for_each_pane(second, f);
        }
    }
}

fn clear_sessions(node: &mut Node) {
    match node {
        Node::Pane(p) => {
            p.session = None;
            p.held = false;
        }
        Node::Split { first, second, .. } => {
            clear_sessions(first);
            clear_sessions(second);
        }
    }
}

/// Loads and saves `layout.json`.
pub struct LayoutFile {
    path: PathBuf,
    last_saved: Mutex<Option<String>>,
}

impl LayoutFile {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_saved: Mutex::new(None),
        }
    }

    pub fn load(&self) -> Option<Layout> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        match serde_json::from_str::<Layout>(&text) {
            Ok(mut l) => {
                l.windows.retain(|w| !w.tabs.is_empty());
                Some(l)
            }
            Err(e) => {
                log::warn!("layout.json is invalid, ignoring: {e}");
                None
            }
        }
    }

    /// Saves unless nothing changed since the last save.
    pub fn save(&self, layout: &Layout) -> CoreResult<()> {
        let text = serde_json::to_string_pretty(&layout.persistable())
            .map_err(|e| CoreError::storage("Не удалось сериализовать раскладку", e))?;
        let mut last = self.last_saved.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        atomic_write(&self.path, text.as_bytes())
            .map_err(|e| CoreError::storage("Не удалось сохранить layout.json", e))?;
        *last = Some(text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> Option<ConnectSpec> {
        Some(ConnectSpec::from_target(
            &format!("u@{name}").parse().unwrap(),
            "k".into(),
            None,
        ))
    }

    fn layout() -> Layout {
        let mut l = Layout::default();
        l.add_window("main", None);
        l
    }

    #[test]
    fn tabs_and_splits() {
        let mut l = layout();
        let (t1, p1) = l.new_tab("main", spec("a"), None).unwrap();
        let p2 = l.split(p1, Direction::Horizontal, spec("b")).unwrap();
        let p3 = l.split(p2, Direction::Vertical, spec("c")).unwrap();
        let p4 = l.split(p1, Direction::Vertical, spec("d")).unwrap();
        let tab = l.tab(t1).unwrap();
        let ids: Vec<_> = tab.panes().iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            vec![p1, p4, p2, p3],
            "tree order: left column then right column"
        );
        assert_eq!(tab.focused, p4);

        // Closing a pane promotes its sibling.
        l.set_session(p2, Some(7)).unwrap();
        let removed = l.close_pane(p2).unwrap();
        assert_eq!(removed.sessions, vec![7]);
        let ids: Vec<_> = l.tab(t1).unwrap().panes().iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![p1, p4, p3]);

        for p in [p1, p4] {
            assert_eq!(l.close_pane(p).unwrap().window_empty, None);
        }
        assert_eq!(
            l.close_pane(p3).unwrap().window_empty.as_deref(),
            Some("main")
        );
        assert!(l.window("main").unwrap().tabs.is_empty());
    }

    #[test]
    fn ratio_is_clamped() {
        let mut l = layout();
        let (t, p) = l.new_tab("main", None, None).unwrap();
        l.split(p, Direction::Horizontal, None).unwrap();
        let Node::Split { id, .. } = l.tab(t).unwrap().root.clone() else {
            panic!()
        };
        l.set_ratio(t, id, 2.0).unwrap();
        let Node::Split { ratio, .. } = l.tab(t).unwrap().root.clone() else {
            panic!()
        };
        assert_eq!(ratio, 0.95);
    }

    #[test]
    fn move_and_reorder_tabs_keep_sessions() {
        let mut l = layout();
        l.add_window("w2", None);
        let (a, pa) = l.new_tab("main", spec("a"), None).unwrap();
        let (b, _) = l.new_tab("main", spec("b"), None).unwrap();
        let (c, _) = l.new_tab("main", spec("c"), None).unwrap();
        l.set_session(pa, Some(42)).unwrap();

        // Reorder within the window: move `a` after `c`.
        l.move_tab(a, "main", Some(3)).unwrap();
        let order: Vec<_> = l
            .window("main")
            .unwrap()
            .tabs
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(order, vec![b, c, a]);

        // Move to another window; session id is kept.
        assert_eq!(l.move_tab(a, "w2", None).unwrap(), None);
        assert_eq!(l.pane(pa).unwrap().session, Some(42));
        assert_eq!(l.pane_window(pa), Some("w2"));
        assert_eq!(l.window("w2").unwrap().active, Some(a));

        // Moving the last tab out reports the empty source window.
        assert_eq!(
            l.move_tab(a, "main", Some(0)).unwrap(),
            Some("w2".to_owned())
        );
        assert_eq!(l.window("main").unwrap().tabs[0].id, a);
    }

    #[test]
    fn close_tab_activates_neighbor() {
        let mut l = layout();
        let (a, _) = l.new_tab("main", None, None).unwrap();
        let (b, _) = l.new_tab("main", None, None).unwrap();
        l.activate_tab(b).unwrap();
        l.close_tab(b).unwrap();
        assert_eq!(l.window("main").unwrap().active, Some(a));
    }

    #[test]
    fn persistence_strips_sessions_and_continues_ids() {
        let dir = crate::paths::test_dir::TestDir::new();
        let file = LayoutFile::new(dir.path().join("layout.json"));
        let mut l = layout();
        let (_, p) = l.new_tab("main", spec("a"), None).unwrap();
        l.set_session(p, Some(5)).unwrap();
        l.set_pane_project(p, Some(" D:\\work\\app\\ ".into()))
            .unwrap();
        l.set_bounds(
            "main",
            Bounds {
                x: 1,
                y: 2,
                width: 800,
                height: 600,
            },
            false,
        );
        file.save(&l).unwrap();

        let mut loaded = file.load().unwrap();
        assert_eq!(loaded.pane(p).unwrap().session, None);
        assert_eq!(loaded.pane(p).unwrap().spec, spec("a"));
        assert_eq!(
            loaded.pane(p).unwrap().project.as_deref(),
            Some("D:\\work\\app")
        );
        assert_eq!(loaded.window("main").unwrap().bounds.unwrap().width, 800);
        let (t2, p2) = loaded.new_tab("main", None, None).unwrap();
        assert!(t2 > p && p2 > t2, "ids continue after the loaded ones");
    }
}
