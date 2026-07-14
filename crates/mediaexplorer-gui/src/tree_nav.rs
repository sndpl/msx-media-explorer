//! Pure keyboard-navigation logic for the file tree.
//!
//! Flattening the tree into the rows currently on screen (respecting collapsed
//! directories) and resolving an arrow key into a cursor move or a
//! collapse/expand. No egui here, so every case is unit-testable; the egui glue
//! in `app.rs` only captures keys and applies the [`TreeNav`] this returns.

use msx_disk::DirEntry;
use std::collections::BTreeSet;

/// One row as drawn in the tree, in top-to-bottom visible order. A directory
/// precedes its children (pre-order), matching the on-screen layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleRow {
    pub path: String,
    pub is_dir: bool,
    /// Nesting level (0 at the root), used for indentation and to find the
    /// first child of an expanded directory.
    pub depth: usize,
    /// The containing directory's path, or `None` for a top-level row.
    pub parent: Option<String>,
    /// Whether the row can be collapsed/expanded. The synthetic disk root is
    /// not collapsible (a disk has exactly one root), so Left never collapses it.
    pub collapsible: bool,
}

/// An arrow key pressed while the tree has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    Left,
    Right,
}

/// What an arrow key resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeNav {
    /// Move the cursor (highlight) to this row's path; if it is a file the
    /// caller also loads it.
    MoveTo(String),
    /// Collapse (`true`) or expand (`false`) this directory; cursor unchanged.
    SetCollapsed(String, bool),
    /// Nothing to do (e.g. an edge of the list, or Right on a file).
    Nothing,
}

/// Flatten `entries` into the rows currently visible, skipping the children of
/// any directory whose path is in `collapsed`.
///
/// When `root` is `Some(path)`, a synthetic root row (e.g. the disk's `/`) is
/// placed first and `entries` become its children (depth 1, parent = `path`);
/// collapsing the root path hides them all. When `root` is `None` the entries
/// render flat at depth 0 (used for partition lists, which have no single root).
///
/// When `keep` is `Some(set)` a filter is active: only entries whose path is in
/// the set are shown, and directories are descended regardless of `collapsed`
/// (a kept directory is an ancestor of a match, so its matches must be revealed).
/// `keep` must not be empty — callers short-circuit an empty keep-set to a
/// "no matches" state. The synthetic root row is always shown.
pub fn flatten_visible(
    entries: &[DirEntry],
    collapsed: &BTreeSet<String>,
    root: Option<&str>,
    keep: Option<&BTreeSet<String>>,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    match root {
        Some(root_path) => {
            rows.push(VisibleRow {
                path: root_path.to_string(),
                is_dir: true,
                depth: 0,
                parent: None,
                collapsible: false,
            });
            // The root is always expanded (not collapsible), so its children
            // show regardless of the collapsed set.
            push_rows(entries, collapsed, 1, Some(root_path), keep, &mut rows);
        }
        None => push_rows(entries, collapsed, 0, None, keep, &mut rows),
    }
    rows
}

fn push_rows(
    entries: &[DirEntry],
    collapsed: &BTreeSet<String>,
    depth: usize,
    parent: Option<&str>,
    keep: Option<&BTreeSet<String>>,
    rows: &mut Vec<VisibleRow>,
) {
    for entry in entries {
        if keep.is_some_and(|k| !k.contains(&entry.path)) {
            continue;
        }
        rows.push(VisibleRow {
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            depth,
            parent: parent.map(str::to_string),
            collapsible: true,
        });
        // While filtering, descend every kept directory so matches inside a
        // collapsed folder are still revealed.
        let descend = keep.is_some() || !collapsed.contains(&entry.path);
        if entry.is_dir && descend {
            push_rows(
                &entry.children,
                collapsed,
                depth + 1,
                Some(&entry.path),
                keep,
                rows,
            );
        }
    }
}

/// Resolve `key` against the visible `rows` and the current `cursor` path.
///
/// `collapsed` is consulted to tell an expanded directory (Left collapses it,
/// Right steps into it) from a collapsed one (Right expands it, Left leaves it
/// for the parent).
pub fn navigate(
    rows: &[VisibleRow],
    cursor: Option<&str>,
    collapsed: &BTreeSet<String>,
    key: NavKey,
) -> TreeNav {
    if rows.is_empty() {
        return TreeNav::Nothing;
    }
    // No cursor yet, or it points at a row that is no longer visible: any key
    // drops the cursor onto the first row.
    let Some(idx) = cursor.and_then(|c| rows.iter().position(|r| r.path == c)) else {
        return TreeNav::MoveTo(rows[0].path.clone());
    };
    let row = &rows[idx];
    match key {
        NavKey::Down if idx + 1 < rows.len() => TreeNav::MoveTo(rows[idx + 1].path.clone()),
        NavKey::Up if idx > 0 => TreeNav::MoveTo(rows[idx - 1].path.clone()),
        NavKey::Down | NavKey::Up => TreeNav::Nothing,
        NavKey::Right => {
            if !row.is_dir {
                return TreeNav::Nothing;
            }
            if row.collapsible && collapsed.contains(&row.path) {
                TreeNav::SetCollapsed(row.path.clone(), false)
            } else if rows.get(idx + 1).is_some_and(|next| next.depth > row.depth) {
                TreeNav::MoveTo(rows[idx + 1].path.clone())
            } else {
                TreeNav::Nothing
            }
        }
        NavKey::Left => {
            if row.is_dir && row.collapsible && !collapsed.contains(&row.path) {
                TreeNav::SetCollapsed(row.path.clone(), true)
            } else if let Some(parent) = &row.parent {
                TreeNav::MoveTo(parent.clone())
            } else {
                TreeNav::Nothing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use msx_disk::fs::Attributes;

    fn file(path: &str) -> DirEntry {
        DirEntry {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            is_dir: false,
            size: 0,
            attributes: Attributes::default(),
            modified: None,
            children: Vec::new(),
        }
    }

    fn dir(path: &str, children: Vec<DirEntry>) -> DirEntry {
        DirEntry {
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            is_dir: true,
            size: 0,
            attributes: Attributes::default(),
            modified: None,
            children,
        }
    }

    /// GAMES/ { A.PCT, B.PCT }, UTILS/ { SUB/ { C.COM } }, HELLO.BAS
    fn sample() -> Vec<DirEntry> {
        vec![
            dir("GAMES", vec![file("GAMES/A.PCT"), file("GAMES/B.PCT")]),
            dir(
                "UTILS",
                vec![dir("UTILS/SUB", vec![file("UTILS/SUB/C.COM")])],
            ),
            file("HELLO.BAS"),
        ]
    }

    fn paths(rows: &[VisibleRow]) -> Vec<&str> {
        rows.iter().map(|r| r.path.as_str()).collect()
    }

    #[test]
    fn flatten_lists_everything_when_nothing_collapsed() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        assert_eq!(
            paths(&rows),
            vec![
                "GAMES",
                "GAMES/A.PCT",
                "GAMES/B.PCT",
                "UTILS",
                "UTILS/SUB",
                "UTILS/SUB/C.COM",
                "HELLO.BAS",
            ]
        );
        // Depth and parent are tracked for indentation / Left-to-parent.
        let sub = rows.iter().find(|r| r.path == "UTILS/SUB").unwrap();
        assert_eq!(sub.depth, 1);
        assert_eq!(sub.parent.as_deref(), Some("UTILS"));
        let leaf = rows.iter().find(|r| r.path == "UTILS/SUB/C.COM").unwrap();
        assert_eq!(leaf.depth, 2);
        assert_eq!(leaf.parent.as_deref(), Some("UTILS/SUB"));
    }

    #[test]
    fn flatten_with_root_nests_entries_under_it() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), Some(""), None);
        // The synthetic root comes first: a depth-0 directory, no parent, and
        // not collapsible (a disk has exactly one root).
        assert_eq!(rows[0].path, "");
        assert_eq!(rows[0].depth, 0);
        assert!(rows[0].is_dir);
        assert_eq!(rows[0].parent, None);
        assert!(!rows[0].collapsible);
        // Former top-level entries are now the root's children.
        let games = rows.iter().find(|r| r.path == "GAMES").unwrap();
        assert_eq!(games.depth, 1);
        assert_eq!(games.parent.as_deref(), Some(""));
        assert!(games.collapsible);
    }

    #[test]
    fn root_is_not_collapsible_so_children_always_show() {
        // Even if the root path were in the collapsed set, its children show.
        let collapsed = BTreeSet::from([String::new()]);
        let rows = flatten_visible(&sample(), &collapsed, Some(""), None);
        assert!(rows.iter().any(|r| r.path == "GAMES"));
        assert!(rows.len() > 1);
    }

    #[test]
    fn navigate_around_the_root_row() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), Some(""), None);
        let c = BTreeSet::new();
        // Right steps from root into its first child.
        assert_eq!(
            navigate(&rows, Some(""), &c, NavKey::Right),
            TreeNav::MoveTo("GAMES".to_string())
        );
        // Left does nothing on the root: it has no parent and cannot collapse.
        assert_eq!(
            navigate(&rows, Some(""), &c, NavKey::Left),
            TreeNav::Nothing
        );
        // Left from a top-level entry returns to the root.
        assert_eq!(
            navigate(&rows, Some("HELLO.BAS"), &c, NavKey::Left),
            TreeNav::MoveTo(String::new())
        );
        // Up from the first entry lands on the root; Up from root does nothing.
        assert_eq!(
            navigate(&rows, Some("GAMES"), &c, NavKey::Up),
            TreeNav::MoveTo(String::new())
        );
        assert_eq!(navigate(&rows, Some(""), &c, NavKey::Up), TreeNav::Nothing);
    }

    #[test]
    fn flatten_hides_children_of_collapsed_dirs() {
        let collapsed = BTreeSet::from(["GAMES".to_string()]);
        let rows = flatten_visible(&sample(), &collapsed, None, None);
        assert_eq!(
            paths(&rows),
            vec![
                "GAMES",
                "UTILS",
                "UTILS/SUB",
                "UTILS/SUB/C.COM",
                "HELLO.BAS"
            ]
        );
    }

    #[test]
    fn filter_keeps_only_matches_and_their_ancestors() {
        // Keep C.COM and its ancestor chain; everything else is hidden.
        let keep = BTreeSet::from([
            "UTILS".to_string(),
            "UTILS/SUB".to_string(),
            "UTILS/SUB/C.COM".to_string(),
        ]);
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, Some(&keep));
        assert_eq!(paths(&rows), vec!["UTILS", "UTILS/SUB", "UTILS/SUB/C.COM"]);
    }

    #[test]
    fn filter_descends_collapsed_directories() {
        // UTILS is collapsed, but a filter must still reveal the match inside it.
        let collapsed = BTreeSet::from(["UTILS".to_string(), "UTILS/SUB".to_string()]);
        let keep = BTreeSet::from([
            "UTILS".to_string(),
            "UTILS/SUB".to_string(),
            "UTILS/SUB/C.COM".to_string(),
        ]);
        let rows = flatten_visible(&sample(), &collapsed, None, Some(&keep));
        assert_eq!(paths(&rows), vec!["UTILS", "UTILS/SUB", "UTILS/SUB/C.COM"]);
    }

    #[test]
    fn filter_with_root_keeps_root_row() {
        let keep = BTreeSet::from(["HELLO.BAS".to_string()]);
        let rows = flatten_visible(&sample(), &BTreeSet::new(), Some(""), Some(&keep));
        assert_eq!(paths(&rows), vec!["", "HELLO.BAS"]);
    }

    #[test]
    fn down_and_up_step_through_visible_rows() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        let c = BTreeSet::new();
        assert_eq!(
            navigate(&rows, Some("GAMES"), &c, NavKey::Down),
            TreeNav::MoveTo("GAMES/A.PCT".to_string())
        );
        assert_eq!(
            navigate(&rows, Some("GAMES/A.PCT"), &c, NavKey::Up),
            TreeNav::MoveTo("GAMES".to_string())
        );
    }

    #[test]
    fn up_and_down_stop_at_the_edges() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        let c = BTreeSet::new();
        assert_eq!(
            navigate(&rows, Some("GAMES"), &c, NavKey::Up),
            TreeNav::Nothing
        );
        assert_eq!(
            navigate(&rows, Some("HELLO.BAS"), &c, NavKey::Down),
            TreeNav::Nothing
        );
    }

    #[test]
    fn no_cursor_drops_onto_the_first_row() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        let c = BTreeSet::new();
        assert_eq!(
            navigate(&rows, None, &c, NavKey::Down),
            TreeNav::MoveTo("GAMES".to_string())
        );
        // A stale cursor (row no longer visible) is treated the same way.
        assert_eq!(
            navigate(&rows, Some("GONE.TXT"), &c, NavKey::Up),
            TreeNav::MoveTo("GAMES".to_string())
        );
    }

    #[test]
    fn right_expands_a_collapsed_dir_then_steps_into_it() {
        let collapsed = BTreeSet::from(["GAMES".to_string()]);
        let rows = flatten_visible(&sample(), &collapsed, None, None);
        // Collapsed: Right expands it (cursor stays).
        assert_eq!(
            navigate(&rows, Some("GAMES"), &collapsed, NavKey::Right),
            TreeNav::SetCollapsed("GAMES".to_string(), false)
        );
        // Once expanded, Right steps onto the first child.
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        assert_eq!(
            navigate(&rows, Some("GAMES"), &BTreeSet::new(), NavKey::Right),
            TreeNav::MoveTo("GAMES/A.PCT".to_string())
        );
    }

    #[test]
    fn right_on_a_file_does_nothing() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        assert_eq!(
            navigate(&rows, Some("HELLO.BAS"), &BTreeSet::new(), NavKey::Right),
            TreeNav::Nothing
        );
    }

    #[test]
    fn left_collapses_an_expanded_dir() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        assert_eq!(
            navigate(&rows, Some("GAMES"), &BTreeSet::new(), NavKey::Left),
            TreeNav::SetCollapsed("GAMES".to_string(), true)
        );
    }

    #[test]
    fn left_jumps_to_parent_from_a_file_or_collapsed_dir() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        let c = BTreeSet::new();
        // A file jumps to its containing directory.
        assert_eq!(
            navigate(&rows, Some("GAMES/A.PCT"), &c, NavKey::Left),
            TreeNav::MoveTo("GAMES".to_string())
        );
        // A collapsed dir jumps to its parent.
        let collapsed = BTreeSet::from(["UTILS/SUB".to_string()]);
        let rows = flatten_visible(&sample(), &collapsed, None, None);
        assert_eq!(
            navigate(&rows, Some("UTILS/SUB"), &collapsed, NavKey::Left),
            TreeNav::MoveTo("UTILS".to_string())
        );
    }

    #[test]
    fn left_at_top_level_does_nothing() {
        let rows = flatten_visible(&sample(), &BTreeSet::new(), None, None);
        // HELLO.BAS is top-level (no parent), and so is a collapsed top dir.
        assert_eq!(
            navigate(&rows, Some("HELLO.BAS"), &BTreeSet::new(), NavKey::Left),
            TreeNav::Nothing
        );
        let collapsed = BTreeSet::from(["GAMES".to_string()]);
        let rows = flatten_visible(&sample(), &collapsed, None, None);
        assert_eq!(
            navigate(&rows, Some("GAMES"), &collapsed, NavKey::Left),
            TreeNav::Nothing
        );
    }
}
