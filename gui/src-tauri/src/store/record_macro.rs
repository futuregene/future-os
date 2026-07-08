//! `sql_record!` — declare a table's `SELECT` column list and its row-mapping
//! function from a *single* ordered field list, so the two can never drift.
//!
//! Previously every record kept a hand-written `*_COLUMNS` string constant and a
//! parallel `*_from_row` with positional `row.get(0)…row.get(N)`. Adding a column
//! to one but not the other — or in a different order — silently mis-mapped every
//! field after the gap. Here both are generated from one list:
//!
//! ```ignore
//! sql_record!(pub(super) RUN_COLUMNS, run_from_row -> RunRecord {
//!     id, thread_id, status, created_at, updated_at,
//! });
//! ```
//!
//! The generated `*_from_row` binds each column positionally into the identically
//! named struct field, so:
//! - the column string and the `get` indices are the same list — no drift;
//! - the struct literal checks the list against the struct: a missing field is a
//!   "missing field" error, an extra one an "unknown field" error.
//!
//! Requirements (hold for every current record): each identifier is both the SQL
//! column name and the struct field name, listed in `SELECT` order, and the field
//! type implements `rusqlite::types::FromSql` (bools map from 0/1 integers).

macro_rules! sql_record {
    ($vis:vis $columns:ident, $from_row:ident -> $record:ident {
        $first:ident $(, $rest:ident)* $(,)?
    }) => {
        /// Comma-separated `SELECT` column list, generated with `sql_record!`.
        $vis const $columns: &str = concat!(stringify!($first) $(, ", ", stringify!($rest))*);

        /// Map a row (selected in `$columns` order) into `$record`, generated with
        /// `sql_record!`.
        $vis fn $from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<$record> {
            #[allow(unused_assignments)]
            {
                let mut column = 0usize;
                let $first = row.get(column)?;
                column += 1;
                $(
                    let $rest = row.get(column)?;
                    column += 1;
                )*
                Ok($record { $first $(, $rest)* })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    #[derive(Debug, PartialEq)]
    struct Toy {
        id: String,
        count: i64,
        flag: bool,
        note: Option<String>,
    }

    sql_record!(TOY_COLUMNS, toy_from_row -> Toy { id, count, flag, note });

    #[test]
    fn columns_track_field_order_and_from_row_maps_positionally() {
        // The generated column string is the field list, comma-joined.
        assert_eq!(TOY_COLUMNS, "id, count, flag, note");

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE toy (id TEXT, count INTEGER, flag INTEGER, note TEXT);
             INSERT INTO toy VALUES ('a', 7, 1, NULL);",
        )
        .unwrap();

        let got = conn
            .query_row(&format!("SELECT {TOY_COLUMNS} FROM toy"), [], toy_from_row)
            .unwrap();
        assert_eq!(
            got,
            Toy {
                id: "a".to_string(),
                count: 7,
                flag: true, // bool maps from the stored 0/1 integer
                note: None, // Option maps NULL
            }
        );
    }
}
