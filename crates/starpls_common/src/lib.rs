use starpls_syntax::{line_index as syntax_line_index, parse_module, LineIndex, Module, Parse};

pub use crate::diagnostics::{Diagnostic, Diagnostics, FileRange, Severity};

mod diagnostics;
mod util;

#[salsa::jar(db = Db)]
pub struct Jar(
    Diagnostics,
    File,
    LineIndexResult,
    ParseResult,
    parse,
    line_index,
);

/// A Key corresponding to an interned file path. Use these instead of `Path`s to refer to files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

/// The base Salsa database. Supports file-related operations, like getting/setting file contents.
pub trait Db: salsa::DbWithJar<Jar> {
    fn set_file_contents(&mut self, file_id: FileId, contents: String) -> File;

    fn get_file(&self, file_id: FileId) -> Option<File>;
}

#[salsa::input]
pub struct File {
    pub id: FileId,
    #[return_ref]
    pub contents: String,
}

#[salsa::tracked]
pub struct ParseResult {
    pub inner: Parse<Module>,
}

#[salsa::tracked]
pub fn parse(db: &dyn Db, file: File) -> ParseResult {
    let parse = parse_module(&file.contents(db), &mut |err| {
        eprintln!("push error");
        Diagnostics::push(
            db,
            Diagnostic {
                message: err.message,
                range: FileRange {
                    file_id: file.id(db),
                    range: err.range,
                },
                severity: Severity::Error,
            },
        )
    });

    ParseResult::new(db, parse)
}

#[salsa::tracked]
pub struct LineIndexResult {
    pub inner: LineIndex,
}

#[salsa::tracked]
pub fn line_index(db: &dyn Db, file: File) -> LineIndexResult {
    let line_index = syntax_line_index(&file.contents(db));
    LineIndexResult::new(db, line_index)
}