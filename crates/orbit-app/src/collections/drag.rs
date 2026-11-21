//! Where a drop lands and whether it changes anything: the pure part of `collection-drag-drop-utils.ts`.

use super::model::{
    is_after_folder_as_root_drop, last_visible_descendant_id, parent_id, root_ancestor_id,
    DropMode, Library, Playlist,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeKind {
    Library,
    Playlist,
}

/// The drop mode for a pointer `offset` below the top of a row of `height`: the top 30 % is before, a folder's
/// middle 40 % is inside, and the bottom 35 % of the last row of a folder's subtree escapes to the root.
pub fn tree_drop_mode(
    offset: f32,
    height: f32,
    kind: TreeKind,
    is_folder: bool,
    after_folder_as_root: bool,
) -> DropMode {
    let mut mode = if offset < height * 0.3 {
        DropMode::Before
    } else {
        DropMode::After
    };
    if kind == TreeKind::Playlist && is_folder && offset >= height * 0.3 && offset <= height * 0.7 {
        mode = DropMode::Inside;
    }
    if kind == TreeKind::Playlist
        && mode == DropMode::After
        && offset >= height * 0.65
        && after_folder_as_root
    {
        mode = DropMode::AfterRoot;
    }
    mode
}

/// Whether the target row is the last row of a nested subtree, so the root can be reached from its bottom edge.
pub fn after_folder_as_root(playlists: &[Playlist], target: &str) -> bool {
    is_after_folder_as_root_drop(playlists, target)
}

/// `isAdjacentNoopDrop`; a missing row has index -1, as in TypeScript.
fn adjacent_noop(dragged: Option<usize>, target: Option<usize>, mode: DropMode) -> bool {
    let index = |position: Option<usize>| position.map_or(-1, |position| position as i64);
    let (dragged, target) = (index(dragged), index(target));
    (mode == DropMode::Before && dragged + 1 == target)
        || (mode == DropMode::After && dragged - 1 == target)
}

pub fn is_noop_library_drop(
    libraries: &[Library],
    dragged: &str,
    target: &str,
    mode: DropMode,
) -> bool {
    if dragged.is_empty() || dragged == target {
        return true;
    }
    adjacent_noop(
        libraries.iter().position(|library| library.id == dragged),
        libraries.iter().position(|library| library.id == target),
        mode,
    )
}

pub fn is_noop_playlist_drop(
    playlists: &[Playlist],
    dragged: &str,
    target: &str,
    mode: DropMode,
) -> bool {
    if dragged.is_empty() || dragged == target {
        return true;
    }
    let Some(dragged_playlist) = playlists.iter().find(|playlist| playlist.id == dragged) else {
        return true;
    };
    let target_playlist = playlists.iter().find(|playlist| playlist.id == target);
    match mode {
        DropMode::Inside => dragged_playlist.parent_id == target,
        DropMode::AfterRoot => {
            let root = root_ancestor_id(playlists, target);
            dragged_playlist.parent_id.is_empty()
                && last_visible_descendant_id(playlists, &root) == dragged
        }
        _ => {
            let Some(target_playlist) = target_playlist else {
                return false;
            };
            if dragged_playlist.parent_id != target_playlist.parent_id {
                return false;
            }
            let siblings: Vec<&Playlist> = playlists
                .iter()
                .filter(|playlist| {
                    parent_id(playlists, playlist) == target_playlist.parent_id.as_str()
                })
                .collect();
            adjacent_noop(
                siblings.iter().position(|playlist| playlist.id == dragged),
                siblings.iter().position(|playlist| playlist.id == target),
                mode,
            )
        }
    }
}

/// A reorder that leaves the order as it is, so the insertion line stays hidden.
pub fn is_entry_noop_drop(sources: &[usize], drop_index: usize) -> bool {
    let mut sorted: Vec<usize> = sources.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    !sorted.is_empty()
        && sorted.windows(2).all(|pair| pair[1] == pair[0] + 1)
        && drop_index == sorted[0]
}
