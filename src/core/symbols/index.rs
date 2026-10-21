use std::sync::Arc;

use diffy_syntax::Symbol;

#[derive(Debug, Clone)]
pub struct FileSymbols {
    pub path: Arc<str>,
    pub symbols: Arc<[Symbol]>,
}

#[derive(Debug, Default)]
pub struct SymbolIndex {
    files: Vec<FileSymbols>,
    by_name: Vec<(u32, u32)>,
    pending: bool,
}

impl SymbolIndex {
    pub fn new(mut files: Vec<FileSymbols>, pending: bool) -> Self {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut by_name: Vec<(u32, u32)> = files
            .iter()
            .enumerate()
            .flat_map(|(file, symbols)| {
                (0..symbols.symbols.len()).map(move |symbol| (file as u32, symbol as u32))
            })
            .collect();
        by_name.sort_by(|a, b| name_of(&files, *a).cmp(name_of(&files, *b)));
        Self {
            files,
            by_name,
            pending,
        }
    }

    pub fn files(&self) -> &[FileSymbols] {
        &self.files
    }

    pub fn file(&self, path: &str) -> Option<&FileSymbols> {
        self.files
            .binary_search_by(|file| (*file.path).cmp(path))
            .ok()
            .map(|index| &self.files[index])
    }

    pub fn symbol_count(&self) -> usize {
        self.by_name.len()
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn definitions<'a>(
        &'a self,
        name: &str,
    ) -> impl Iterator<Item = (&'a FileSymbols, &'a Symbol)> + 'a {
        let start = self
            .by_name
            .partition_point(|entry| name_of(&self.files, *entry) < name);
        let end = start
            + self.by_name[start..].partition_point(|entry| name_of(&self.files, *entry) == name);
        self.by_name[start..end].iter().map(|&(file, symbol)| {
            let file = &self.files[file as usize];
            (file, &file.symbols[symbol as usize])
        })
    }
}

fn name_of(files: &[FileSymbols], (file, symbol): (u32, u32)) -> &str {
    &files[file as usize].symbols[symbol as usize].name
}

#[cfg(test)]
mod tests {
    use diffy_syntax::SymbolKind;

    use super::*;

    fn file(path: &str, names: &[&str]) -> FileSymbols {
        FileSymbols {
            path: Arc::from(path),
            symbols: names
                .iter()
                .enumerate()
                .map(|(line, name)| Symbol {
                    name: Box::from(*name),
                    kind: SymbolKind::Function,
                    line: line as u32,
                    column: 0,
                    start_line: line as u32,
                    end_line: line as u32,
                })
                .collect(),
        }
    }

    #[test]
    fn finds_definitions_by_exact_name_across_files() {
        let index = SymbolIndex::new(
            vec![
                file("src/b.rs", &["run", "stop"]),
                file("src/a.rs", &["start", "run", "runner"]),
            ],
            false,
        );
        let found: Vec<(&str, u32)> = index
            .definitions("run")
            .map(|(file, symbol)| (&*file.path, symbol.line))
            .collect();
        assert_eq!(found, vec![("src/a.rs", 1), ("src/b.rs", 0)]);
        assert_eq!(index.definitions("missing").count(), 0);
        assert_eq!(index.file("src/b.rs").map(|f| f.symbols.len()), Some(2));
        assert_eq!(index.symbol_count(), 5);
    }
}
