//! `concord diff`: what changed between two trees, as a unified diff per path.

use std::io;

use similar::TextDiff;

use crate::objects::{Entry, Objects, Tree};

/// Unified diff from `from` to `to`, paths in order. A file that is not UTF-8 (or a symlink on
/// either side) is named, not shown.
pub fn unified(objects: &Objects, from: &Tree, to: &Tree) -> io::Result<String> {
    let mut paths: Vec<&String> = from.keys().chain(to.keys()).collect();
    paths.sort();
    paths.dedup();
    let mut out = String::new();
    for path in paths {
        let (a, b) = (from.get(path), to.get(path));
        if a == b {
            continue;
        }
        let old = text(objects, a)?;
        let new = text(objects, b)?;
        let (old_name, new_name) = (
            a.map_or("/dev/null".to_string(), |_| format!("a/{path}")),
            b.map_or("/dev/null".to_string(), |_| format!("b/{path}")),
        );
        match (old, new) {
            (Some(old), Some(new)) => {
                let diff = TextDiff::from_lines(&old, &new);
                out.push_str(&format!("diff {path}\n"));
                out.push_str(
                    &diff
                        .unified_diff()
                        .header(&old_name, &new_name)
                        .to_string(),
                );
                if a.map(|e| e.exec) != b.map(|e| e.exec) && a.is_some() && b.is_some() {
                    out.push_str(&format!("mode of {path}: exec {}\n", b.is_some_and(|e| e.exec)));
                }
            }
            _ => out.push_str(&format!("diff {path}: binary or symlink, differs\n")),
        }
    }
    Ok(out)
}

/// The entry's content as text; `Some("")` for no entry, `None` when it cannot be shown as text.
fn text(objects: &Objects, entry: Option<&Entry>) -> io::Result<Option<String>> {
    let Some(entry) = entry else {
        return Ok(Some(String::new()));
    };
    if entry.link {
        return Ok(None);
    }
    Ok(String::from_utf8(objects.get(&entry.blob)?).ok())
}
