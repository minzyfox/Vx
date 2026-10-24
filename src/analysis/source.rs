use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineColumn {
    /// One-based line number.
    pub line: usize,
    /// One-based Unicode character column, as used by the lexer.
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LspPosition {
    /// Zero-based line number.
    pub line: usize,
    /// Zero-based UTF-16 code unit offset.
    pub character: usize,
}

#[derive(Debug)]
pub struct SourceFile {
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    fn new(path: PathBuf, text: String) -> Self {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self {
            path,
            text,
            line_starts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        let start = *self.line_starts.get(line)?;
        let mut end = self
            .line_starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1);
        if end > start && self.text.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        Some((start, end))
    }

    fn line_at_byte(&self, offset: usize) -> Option<(usize, usize)> {
        if !self.text.is_char_boundary(offset) {
            return None;
        }
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let (start, end) = self.line_bounds(line)?;
        (offset <= end).then_some((line, start))
    }

    pub fn byte_to_line_column(&self, offset: usize) -> Option<LineColumn> {
        let (line, start) = self.line_at_byte(offset)?;
        Some(LineColumn {
            line: line + 1,
            column: self.text[start..offset].chars().count() + 1,
        })
    }

    pub fn line_column_to_byte(&self, position: LineColumn) -> Option<usize> {
        let line = position.line.checked_sub(1)?;
        let column = position.column.checked_sub(1)?;
        let (start, end) = self.line_bounds(line)?;
        let text = &self.text[start..end];
        if column == text.chars().count() {
            return Some(end);
        }
        text.char_indices()
            .nth(column)
            .map(|(offset, _)| start + offset)
    }

    pub fn byte_to_lsp_position(&self, offset: usize) -> Option<LspPosition> {
        let (line, start) = self.line_at_byte(offset)?;
        Some(LspPosition {
            line,
            character: self.text[start..offset].encode_utf16().count(),
        })
    }

    pub fn lsp_position_to_byte(&self, position: LspPosition) -> Option<usize> {
        let (start, end) = self.line_bounds(position.line)?;
        let mut units = 0;
        for (offset, character) in self.text[start..end].char_indices() {
            if units == position.character {
                return Some(start + offset);
            }
            units += character.len_utf16();
            if units > position.character {
                return None;
            }
        }
        (units == position.character).then_some(end)
    }

    pub fn slice(&self, start: usize, end: usize) -> Option<&str> {
        self.text.get(start..end)
    }
}

#[derive(Default)]
pub struct SourceStore {
    files: Vec<SourceFile>,
    by_path: HashMap<PathBuf, SourceId>,
    overrides: HashMap<PathBuf, String>,
}

impl SourceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep the path even when the file does not exist on disk yet.
    pub fn path_key(path: &Path) -> io::Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut clean = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    clean.pop();
                }
                other => clean.push(other.as_os_str()),
            }
        }
        Ok(clean)
    }

    pub fn set_override(&mut self, path: &Path, text: String) -> io::Result<()> {
        self.overrides.insert(Self::path_key(path)?, text);
        Ok(())
    }

    pub fn has_override(&self, path: &Path) -> bool {
        Self::path_key(path).is_ok_and(|key| self.overrides.contains_key(&key))
    }

    pub fn read_text(&self, path: &Path) -> io::Result<String> {
        if let Some(text) = self.overrides.get(&Self::path_key(path)?) {
            return Ok(text.clone());
        }
        fs::read_to_string(path)
    }

    pub fn insert(&mut self, path: &Path, text: String) -> io::Result<SourceId> {
        let path = Self::path_key(path)?;
        if let Some(id) = self.by_path.get(&path).copied() {
            self.files[id.0] = SourceFile::new(path, text);
            return Ok(id);
        }
        let id = SourceId(self.files.len());
        self.files.push(SourceFile::new(path.clone(), text));
        self.by_path.insert(path, id);
        Ok(id)
    }

    pub fn load(&mut self, path: &Path) -> io::Result<SourceId> {
        let text = self.read_text(path)?;
        self.insert(path, text)
    }

    pub fn get(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.0)
    }

    pub fn id_for_path(&self, path: &Path) -> Option<SourceId> {
        Self::path_key(path)
            .ok()
            .and_then(|key| self.by_path.get(&key).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_handle_ascii_unicode_crlf_and_end_of_file() {
        let file = SourceFile::new(PathBuf::from("example.vx"), "aé😀\r\nnext\n".into());
        assert_eq!(file.line_starts(), &[0, 9, 14]);
        assert_eq!(
            file.byte_to_line_column(0),
            Some(LineColumn { line: 1, column: 1 })
        );
        assert_eq!(
            file.byte_to_line_column(1),
            Some(LineColumn { line: 1, column: 2 })
        );
        assert_eq!(
            file.byte_to_line_column(3),
            Some(LineColumn { line: 1, column: 3 })
        );
        assert_eq!(
            file.byte_to_line_column(7),
            Some(LineColumn { line: 1, column: 4 })
        );
        assert_eq!(file.byte_to_line_column(8), None);
        assert_eq!(
            file.byte_to_line_column(14),
            Some(LineColumn { line: 3, column: 1 })
        );
        assert_eq!(
            file.line_column_to_byte(LineColumn { line: 1, column: 3 }),
            Some(3)
        );
        assert_eq!(
            file.line_column_to_byte(LineColumn { line: 3, column: 1 }),
            Some(14)
        );
        assert_eq!(
            file.byte_to_lsp_position(7),
            Some(LspPosition {
                line: 0,
                character: 4
            })
        );
        assert_eq!(
            file.lsp_position_to_byte(LspPosition {
                line: 0,
                character: 3
            }),
            None
        );
        assert_eq!(
            file.lsp_position_to_byte(LspPosition {
                line: 0,
                character: 4
            }),
            Some(7)
        );
        assert_eq!(
            file.lsp_position_to_byte(LspPosition {
                line: 2,
                character: 0
            }),
            Some(14)
        );
        assert_eq!(file.slice(1, 3), Some("é"));
        assert_eq!(file.slice(2, 3), None);
        assert_eq!(file.slice(3, 7), Some("😀"));
    }

    #[test]
    fn equal_text_at_different_paths_has_different_ids() {
        let mut store = SourceStore::new();
        let first = store
            .insert(Path::new("a.vx"), "fn main() {}".into())
            .unwrap();
        let second = store
            .insert(Path::new("b.vx"), "fn main() {}".into())
            .unwrap();
        assert_ne!(first, second);
        assert_ne!(
            store.get(first).unwrap().path(),
            store.get(second).unwrap().path()
        );
    }
}
