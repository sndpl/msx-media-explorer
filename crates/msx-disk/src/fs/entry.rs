//! Plain, UI-free data structures describing the contents of a disk.

/// A DOS-style file attribute set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attributes {
    pub read_only: bool,
    pub hidden: bool,
    pub system: bool,
    pub archive: bool,
}

/// A decoded directory-entry timestamp (minute resolution is plenty for MSX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// One entry (file or directory) in the disk's directory tree.
///
/// Directories carry their `children`, so a single [`DirEntry`] returned from
/// the root is a full recursive tree — including MSX-DOS 2 subdirectories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Bare 8.3 name, e.g. `GAME.COM`.
    pub name: String,
    /// Full slash-separated path from the root, e.g. `UTILS/GAME.COM`.
    pub path: String,
    pub is_dir: bool,
    /// File size in bytes (0 for directories).
    pub size: u64,
    pub attributes: Attributes,
    pub modified: Option<Timestamp>,
    /// Child entries, for directories.
    pub children: Vec<DirEntry>,
}

impl DirEntry {
    /// Depth-first iterator over this entry and all of its descendants.
    pub fn walk(&self) -> impl Iterator<Item = &DirEntry> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            for child in next.children.iter().rev() {
                stack.push(child);
            }
            Some(next)
        })
    }
}
