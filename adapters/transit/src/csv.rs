//! Minimal RFC 4180 reader for the small, pre-filtered GTFS subset files.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CsvError {
    #[error("file is empty")]
    Empty,
    #[error("unterminated quoted field")]
    UnterminatedQuote,
    #[error("missing column {0}")]
    MissingColumn(&'static str),
    #[error("row {row} has {found} fields, header has {expected}")]
    RowWidth {
        row: usize,
        found: usize,
        expected: usize,
    },
}

/// A parsed CSV file: header plus rows of equal width.
#[derive(Debug, Clone)]
pub(crate) struct Table {
    columns: HashMap<String, usize>,
    pub(crate) rows: Vec<Vec<String>>,
}

impl Table {
    pub(crate) fn parse(text: &str) -> Result<Self, CsvError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut records = records(text)?.into_iter();
        let header = records.next().ok_or(CsvError::Empty)?;
        let columns = header
            .iter()
            .enumerate()
            .map(|(i, name)| (name.trim().to_owned(), i))
            .collect();
        let rows: Vec<Vec<String>> = records.collect();
        for (i, row) in rows.iter().enumerate() {
            if row.len() != header.len() {
                return Err(CsvError::RowWidth {
                    row: i + 2,
                    found: row.len(),
                    expected: header.len(),
                });
            }
        }
        Ok(Self { columns, rows })
    }

    /// Index of a required column.
    pub(crate) fn col(&self, name: &'static str) -> Result<usize, CsvError> {
        self.columns
            .get(name)
            .copied()
            .ok_or(CsvError::MissingColumn(name))
    }

    /// Index of an optional column.
    pub(crate) fn opt_col(&self, name: &str) -> Option<usize> {
        self.columns.get(name).copied()
    }
}

/// Split into records; blank lines are skipped, quotes may span lines.
fn records(text: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let mut out = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => end_record(&mut out, &mut record, &mut field),
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(CsvError::UnterminatedQuote);
    }
    end_record(&mut out, &mut record, &mut field);
    Ok(out)
}

fn end_record(out: &mut Vec<Vec<String>>, record: &mut Vec<String>, field: &mut String) {
    if record.is_empty() && field.is_empty() {
        return;
    }
    record.push(std::mem::take(field));
    out.push(std::mem::take(record));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_crlf_and_bom() {
        let t = Table::parse("\u{feff}a,b\r\n1,\"x, \"\"y\"\"\"\r\n\r\n2,z\n").unwrap();
        assert_eq!(t.col("b").unwrap(), 1);
        assert_eq!(t.rows, vec![vec!["1", "x, \"y\""], vec!["2", "z"]]);
    }

    #[test]
    fn rejects_ragged_rows_and_open_quotes() {
        assert!(matches!(
            Table::parse("a,b\n1\n"),
            Err(CsvError::RowWidth { row: 2, .. })
        ));
        assert_eq!(
            Table::parse("a\n\"open\n").unwrap_err(),
            CsvError::UnterminatedQuote
        );
        assert_eq!(Table::parse("").unwrap_err(), CsvError::Empty);
    }
}
