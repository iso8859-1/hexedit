//! In-memory file buffer with undo/redo.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
enum Op {
    Replace { off: usize, old: Vec<u8>, new: Vec<u8> },
    Insert { off: usize, data: Vec<u8> },
    Delete { off: usize, data: Vec<u8> },
}

#[derive(Debug, Clone)]
struct Edit {
    id: u64,
    ops: Vec<Op>,
}

/// Range touched by an undo/redo, so the UI can move the cursor there.
pub struct Touched {
    pub offset: u64,
    pub len: u64,
}

#[derive(Default)]
pub struct Document {
    pub path: Option<PathBuf>,
    pub data: Vec<u8>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    next_id: u64,
    /// id of the top undo entry when last saved
    saved_id: Option<u64>,
    /// files already backed up in this session; later saves keep the first backup
    backed_up: Vec<PathBuf>,
}

/// `file.ext` -> `file.ext.bak`
pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

impl Document {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let data = fs::read(path)?;
        Ok(Document { path: Some(path.to_path_buf()), data, ..Default::default() })
    }

    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn dirty(&self) -> bool {
        self.undo.last().map(|e| e.id) != self.saved_id
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Saves to `path`. With `backup`, the first save over an existing file in
    /// this session keeps its previous content as `<path>.bak` (returned).
    pub fn save_to(&mut self, path: &Path, backup: bool) -> std::io::Result<Option<PathBuf>> {
        // write next to the target and rename, so a failed write never truncates the file
        let tmp = path.with_extension(format!(
            "{}.hexedit-tmp",
            path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
        ));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&self.data)?;
            f.sync_all()?;
        }
        let bak = if backup && path.is_file() && !self.backed_up.iter().any(|p| p == path) {
            // the original moves aside instead of being copied: instant, even for large files
            let bak = backup_path(path);
            if let Err(e) = fs::rename(path, &bak) {
                let _ = fs::remove_file(&tmp);
                return Err(std::io::Error::new(e.kind(), format!("cannot create backup {}: {e}", bak.display())));
            }
            Some(bak)
        } else {
            None
        };
        if let Err(e) = fs::rename(&tmp, path) {
            if let Some(b) = &bak {
                let _ = fs::rename(b, path);
            }
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        if bak.is_some() {
            self.backed_up.push(path.to_path_buf());
        }
        self.path = Some(path.to_path_buf());
        self.saved_id = self.undo.last().map(|e| e.id);
        Ok(bak)
    }

    /// Writes `bytes` at `off`. In overwrite mode the part past the end of the
    /// file is appended. `coalesce` merges with the previous edit when it
    /// continues it (typing), so one undo reverts the whole run.
    pub fn write(&mut self, off: usize, bytes: &[u8], insert: bool, coalesce: bool) -> Result<(), String> {
        if off > self.data.len() {
            return Err(format!("offset {off} is beyond the end of the file"));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        let ops = if insert {
            vec![Op::Insert { off, data: bytes.to_vec() }]
        } else {
            let in_range = bytes.len().min(self.data.len() - off);
            let mut ops = Vec::new();
            if in_range > 0 {
                ops.push(Op::Replace {
                    off,
                    old: self.data[off..off + in_range].to_vec(),
                    new: bytes[..in_range].to_vec(),
                });
            }
            if in_range < bytes.len() {
                ops.push(Op::Insert { off: off + in_range, data: bytes[in_range..].to_vec() });
            }
            ops
        };
        for op in &ops {
            apply(&mut self.data, op, false);
        }
        if coalesce && ops.len() == 1 && self.try_coalesce(&ops[0]) {
            self.redo.clear();
            return Ok(());
        }
        self.push(ops);
        Ok(())
    }

    pub fn delete(&mut self, off: usize, len: usize) -> Result<(), String> {
        let end = off.checked_add(len).filter(|&e| e <= self.data.len()).ok_or("range is beyond the end of the file")?;
        if len == 0 {
            return Ok(());
        }
        let op = Op::Delete { off, data: self.data[off..end].to_vec() };
        apply(&mut self.data, &op, false);
        self.push(vec![op]);
        Ok(())
    }

    pub fn undo(&mut self) -> Option<Touched> {
        let e = self.undo.pop()?;
        for op in e.ops.iter().rev() {
            apply(&mut self.data, op, true);
        }
        let t = touched(&e.ops, true);
        self.redo.push(e);
        Some(t)
    }

    pub fn redo(&mut self) -> Option<Touched> {
        let e = self.redo.pop()?;
        for op in &e.ops {
            apply(&mut self.data, op, false);
        }
        let t = touched(&e.ops, false);
        self.undo.push(e);
        Some(t)
    }

    fn push(&mut self, ops: Vec<Op>) {
        self.next_id += 1;
        self.undo.push(Edit { id: self.next_id, ops });
        self.redo.clear();
    }

    /// Merges an already applied op into the last undo entry if it continues it.
    fn try_coalesce(&mut self, op: &Op) -> bool {
        // never merge into the state the file was saved in
        if self.dirty_top_is_saved() {
            return false;
        }
        let Some(last) = self.undo.last_mut() else { return false };
        if last.ops.len() != 1 {
            return false;
        }
        match (&mut last.ops[0], op) {
            (Op::Replace { off: lo, old: lold, new: lnew }, Op::Replace { off, old, new }) => {
                if *off < *lo || *off > *lo + lnew.len() {
                    return false;
                }
                for (i, (&o, &n)) in old.iter().zip(new).enumerate() {
                    let pos = *off - *lo + i;
                    if pos < lnew.len() {
                        lnew[pos] = n;
                    } else {
                        lold.push(o);
                        lnew.push(n);
                    }
                }
                true
            }
            (Op::Insert { off: lo, data }, Op::Replace { off, new, .. }) => {
                if *off < *lo || *off + new.len() > *lo + data.len() {
                    return false;
                }
                data[*off - *lo..*off - *lo + new.len()].copy_from_slice(new);
                true
            }
            (Op::Insert { off: lo, data }, Op::Insert { off, data: nd }) => {
                if *off != *lo + data.len() {
                    return false;
                }
                data.extend_from_slice(nd);
                true
            }
            _ => false,
        }
    }

    fn dirty_top_is_saved(&self) -> bool {
        self.undo.last().map(|e| e.id).is_some() && self.undo.last().map(|e| e.id) == self.saved_id
    }
}

fn apply(data: &mut Vec<u8>, op: &Op, reverse: bool) {
    match (op, reverse) {
        (Op::Replace { off, new, .. }, false) => data[*off..*off + new.len()].copy_from_slice(new),
        (Op::Replace { off, old, .. }, true) => data[*off..*off + old.len()].copy_from_slice(old),
        (Op::Insert { off, data: d }, false) | (Op::Delete { off, data: d }, true) => {
            data.splice(*off..*off, d.iter().copied());
        }
        (Op::Insert { off, data: d }, true) | (Op::Delete { off, data: d }, false) => {
            data.drain(*off..*off + d.len());
        }
    }
}

fn touched(ops: &[Op], undo: bool) -> Touched {
    let (off, len) = match &ops[0] {
        Op::Replace { off, new, .. } => (*off, new.len()),
        Op::Insert { off, data } => (*off, if undo { 0 } else { data.len() }),
        Op::Delete { off, data } => (*off, if undo { data.len() } else { 0 }),
    };
    let extra: usize = ops[1..]
        .iter()
        .map(|o| match o {
            Op::Insert { data, .. } if !undo => data.len(),
            _ => 0,
        })
        .sum();
    Touched { offset: off as u64, len: (len + extra) as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_undo_redo() {
        let mut d = Document { data: vec![1, 2, 3, 4], ..Default::default() };
        d.write(1, &[9, 9], false, false).unwrap();
        assert_eq!(d.data, [1, 9, 9, 4]);
        d.write(3, &[7, 7, 7], false, false).unwrap(); // overflows: replace + append
        assert_eq!(d.data, [1, 9, 9, 7, 7, 7]);
        d.delete(0, 2).unwrap();
        assert_eq!(d.data, [9, 7, 7, 7]);
        d.write(1, &[5], true, false).unwrap();
        assert_eq!(d.data, [9, 5, 7, 7, 7]);
        d.undo();
        d.undo();
        assert_eq!(d.data, [1, 9, 9, 7, 7, 7]);
        d.undo();
        d.undo();
        assert_eq!(d.data, [1, 2, 3, 4]);
        assert!(!d.dirty());
        d.redo();
        assert_eq!(d.data, [1, 9, 9, 4]);
        assert!(d.dirty());
    }

    #[test]
    fn save_with_backup() {
        let dir = std::env::temp_dir().join(format!("hexedit-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.bin");
        let bak = backup_path(&path);
        assert_eq!(bak.file_name().unwrap(), "f.bin.bak");
        fs::write(&path, [1, 2, 3]).unwrap();
        let _ = fs::remove_file(&bak);

        let mut d = Document::open(&path).unwrap();
        d.write(0, &[9], false, false).unwrap();
        assert_eq!(d.save_to(&path, true).unwrap(), Some(bak.clone()));
        assert_eq!(fs::read(&path).unwrap(), [9, 2, 3]);
        assert_eq!(fs::read(&bak).unwrap(), [1, 2, 3]);
        // the backup keeps the original, not the previous save
        d.write(1, &[8], false, false).unwrap();
        assert_eq!(d.save_to(&path, true).unwrap(), None);
        assert_eq!(fs::read(&path).unwrap(), [9, 8, 3]);
        assert_eq!(fs::read(&bak).unwrap(), [1, 2, 3]);
        // without the option no backup is made
        let other = dir.join("g.bin");
        fs::write(&other, [0]).unwrap();
        let _ = fs::remove_file(backup_path(&other));
        assert_eq!(d.save_to(&other, false).unwrap(), None);
        assert!(!backup_path(&other).exists());
        // new files have nothing to back up
        let fresh = dir.join("new.bin");
        let _ = fs::remove_file(&fresh);
        assert_eq!(d.save_to(&fresh, true).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn typing_coalesces() {
        let mut d = Document { data: vec![0; 4], ..Default::default() };
        // nibble-by-nibble typing of "AB CD"
        d.write(0, &[0xA0], false, true).unwrap();
        d.write(0, &[0xAB], false, true).unwrap();
        d.write(1, &[0xC0], false, true).unwrap();
        d.write(1, &[0xCD], false, true).unwrap();
        assert_eq!(d.data, [0xAB, 0xCD, 0, 0]);
        d.undo();
        assert_eq!(d.data, [0, 0, 0, 0]);
        assert!(!d.can_undo());

        // insert mode: first nibble inserts, second replaces the inserted byte
        d.write(4, &[0x10], true, true).unwrap();
        d.write(4, &[0x12], false, true).unwrap();
        d.write(5, &[0x30], true, true).unwrap();
        assert_eq!(d.data, [0, 0, 0, 0, 0x12, 0x30]);
        d.undo();
        assert_eq!(d.data, [0, 0, 0, 0]);
    }
}
