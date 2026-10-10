//! 18.6's `quote_identifier` — the rule behind the SQL function `quote_ident(text)` and
//! behind every name `pg_get_serial_sequence` answers — reproduced as a **value**.
//!
//! A catalog rendering's function name and `EXTRACT` field take it too (`deparse`).
//!
//! It is deliberately not [`super::quote_ident`]. That renderer asks pglet's own lexer
//! whether a word is a keyword, and pglet's keyword categories are not 18.6's
//! (`EDGES.md` row 85 (f): `action` is quoted there and bare on 18.6). Where the string
//! is SQL text pglet must reload, an over-quote is free and that renderer's answer is the
//! safe one; where the string *is* the answer a client reads, it must be 18.6's. This
//! module answers from 18.6's own keyword table instead, [`QUOTED_KEYWORDS`].
//!
//! @module

/// Every keyword whose category is **not** `UNRESERVED_KEYWORD` on the 18.6 oracle —
/// the words `quote_identifier` quotes although they are otherwise safe identifiers.
///
/// Read off the running server rather than the source: `SELECT word FROM
/// pg_get_keywords() WHERE catcode <> 'U' ORDER BY word COLLATE "C"` on 18.6 (the
/// PostgreSQL source's development branch tracks a newer major whose keyword list can
/// differ). Sorted by byte, so [`quote_identifier`] searches it; `tests/fixtures/functions/
/// quote_ident_every_keyword.sql` compares the rule over every keyword 18.6 has, all
/// four categories.
pub const QUOTED_KEYWORDS: [&str; 164] = [
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "authorization",
    "between",
    "bigint",
    "binary",
    "bit",
    "boolean",
    "both",
    "case",
    "cast",
    "char",
    "character",
    "check",
    "coalesce",
    "collate",
    "collation",
    "column",
    "concurrently",
    "constraint",
    "create",
    "cross",
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "dec",
    "decimal",
    "default",
    "deferrable",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "exists",
    "extract",
    "false",
    "fetch",
    "float",
    "for",
    "foreign",
    "freeze",
    "from",
    "full",
    "grant",
    "greatest",
    "group",
    "grouping",
    "having",
    "ilike",
    "in",
    "initially",
    "inner",
    "inout",
    "int",
    "integer",
    "intersect",
    "interval",
    "into",
    "is",
    "isnull",
    "join",
    "json",
    "json_array",
    "json_arrayagg",
    "json_exists",
    "json_object",
    "json_objectagg",
    "json_query",
    "json_scalar",
    "json_serialize",
    "json_table",
    "json_value",
    "lateral",
    "leading",
    "least",
    "left",
    "like",
    "limit",
    "localtime",
    "localtimestamp",
    "merge_action",
    "national",
    "natural",
    "nchar",
    "none",
    "normalize",
    "not",
    "notnull",
    "null",
    "nullif",
    "numeric",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "out",
    "outer",
    "overlaps",
    "overlay",
    "placing",
    "position",
    "precision",
    "primary",
    "real",
    "references",
    "returning",
    "right",
    "row",
    "select",
    "session_user",
    "setof",
    "similar",
    "smallint",
    "some",
    "substring",
    "symmetric",
    "system_user",
    "table",
    "tablesample",
    "then",
    "time",
    "timestamp",
    "to",
    "trailing",
    "treat",
    "trim",
    "true",
    "union",
    "unique",
    "user",
    "using",
    "values",
    "varchar",
    "variadic",
    "verbose",
    "when",
    "where",
    "window",
    "with",
    "xmlattributes",
    "xmlconcat",
    "xmlelement",
    "xmlexists",
    "xmlforest",
    "xmlnamespaces",
    "xmlparse",
    "xmlpi",
    "xmlroot",
    "xmlserialize",
    "xmltable",
];

/// `name` as 18.6's `quote_identifier` renders it.
///
/// Bare when its first character is `a`–`z` or `_`, every character is `a`–`z`, `0`–`9` or
/// `_`, and it is not one of [`QUOTED_KEYWORDS`]; otherwise double-quoted with each `"`
/// doubled. The test is
/// byte-wise and ASCII-only, as 18.6's is, so a non-ASCII name is always quoted and the
/// empty name is `""`.
#[must_use]
pub fn quote_identifier(name: &str) -> String {
    let bytes = name.as_bytes();
    let safe = bytes
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || *b == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        && QUOTED_KEYWORDS.binary_search(&name).is_err();
    if safe {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

#[cfg(test)]
mod tests {
    use super::{QUOTED_KEYWORDS, quote_identifier};

    /// The search needs byte order and one entry per word.
    #[test]
    fn the_keyword_table_is_sorted_by_byte_and_unique() {
        assert!(
            QUOTED_KEYWORDS
                .windows(2)
                .all(|w| w[0].as_bytes() < w[1].as_bytes())
        );
    }

    /// 18.6's answers, measured with `SELECT quote_ident(…)`.
    #[test]
    fn answers_as_18_6_does() {
        for (input, want) in [
            ("abc", "abc"),
            ("Abc", "\"Abc\""),
            ("a b", "\"a b\""),
            ("a\"b", "\"a\"\"b\""),
            ("", "\"\""),
            ("1a", "\"1a\""),
            ("_a", "_a"),
            ("a1_", "a1_"),
            ("é", "\"é\""),
            ("a$", "\"a$\""),
            ("select", "\"select\""),
            ("action", "action"),
            ("int", "\"int\""),
            ("left", "\"left\""),
        ] {
            assert_eq!(quote_identifier(input), want, "{input:?}");
        }
    }
}
