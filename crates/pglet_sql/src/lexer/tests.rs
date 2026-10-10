//! Unit + fuzz coverage for the SQL lexer.

use super::token::Keyword;
use super::{LexError, Lexer, Token, TokenKind, tokenize};

/// The SQLSTATE a client is sent for `err`, read through `parser::lex_error_to_error` —
/// the one mapping the parser boundary applies — rather than off a second table. The
/// clone is this helper's: the production path consumes the error.
fn client_sqlstate(err: &LexError, source: &str) -> &'static str {
    crate::parser::lex_error_to_error(err.clone(), source).sqlstate()
}

/// Tokenize, asserting success, and drop the trailing `Eof` for terse kind checks.
fn kinds(sql: &str) -> Vec<TokenKind> {
    let mut kinds: Vec<TokenKind> = tokenize(sql).unwrap().into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds.pop(),
        Some(TokenKind::Eof),
        "stream must end in Eof: {sql:?}"
    );
    kinds
}

/// The single significant token of `sql` (asserts exactly one + `Eof`).
fn one(sql: &str) -> Token {
    let mut tokens = tokenize(sql).unwrap();
    assert_eq!(
        tokens.len(),
        2,
        "expected one token + Eof for {sql:?}: {tokens:?}"
    );
    tokens.swap_remove(0)
}

/// The decoded/verbatim value of the single string-or-identifier token of `sql`.
fn value(sql: &str) -> String {
    let token = one(sql);
    token.text(sql).to_string()
}

fn kw(k: Keyword) -> TokenKind {
    TokenKind::Keyword(k)
}

#[test]
fn keywords_are_case_insensitive() {
    for spelling in ["select", "SELECT", "Select", "sElEcT"] {
        assert_eq!(one(spelling).kind, kw(Keyword::Select), "{spelling:?}");
    }
    assert_eq!(
        kinds("INSERT INTO t VALUES"),
        vec![
            kw(Keyword::Insert),
            kw(Keyword::Into),
            TokenKind::Identifier,
            kw(Keyword::Values)
        ]
    );
}

#[test]
fn identifiers_are_not_overgreedy_keywords() {
    // A keyword must match the whole identifier, not a prefix.
    assert_eq!(one("selected").kind, TokenKind::Identifier);
    assert_eq!(one("value").kind, TokenKind::Identifier); // only `values` is a keyword
    assert_eq!(one("_x9").kind, TokenKind::Identifier);
    assert_eq!(one("excluded").kind, TokenKind::Identifier); // pseudo-table, not reserved
}

#[test]
fn bare_identifier_text_is_verbatim() {
    // Case folding of bare identifiers is the parser's job, not the lexer's.
    assert_eq!(value("MyColumn"), "MyColumn");
    assert_eq!(value("a$b"), "a$b"); // `$` continues an identifier
}

#[test]
fn quoted_identifiers_preserve_case_and_unquote() {
    let token = one("\"My Col\"");
    assert_eq!(token.kind, TokenKind::QuotedIdentifier);
    assert_eq!(token.text("\"My Col\""), "My Col");
    // `""` collapses to a single quote.
    assert_eq!(value("\"a\"\"b\""), "a\"b");
}

#[test]
fn qualified_names_and_wildcards() {
    assert_eq!(
        kinds("c.id"),
        vec![TokenKind::Identifier, TokenKind::Dot, TokenKind::Identifier]
    );
    assert_eq!(
        kinds("t.*"),
        vec![TokenKind::Identifier, TokenKind::Dot, TokenKind::Star]
    );
}

#[test]
fn numbers() {
    for n in ["0", "123", "1.5", ".5", "5.", "1e10", "1.5e-3", "9E+2"] {
        assert_eq!(one(n).kind, TokenKind::Number, "{n:?}");
        assert_eq!(one(n).text(n), n);
    }
    // A bare `.` is a dot, not a number.
    assert_eq!(one(".").kind, TokenKind::Dot);
    // `1e` with no exponent digits glues an identifier byte to the literal —
    // trailing junk, a loud error (Postgres's `integer_junk` rule), not `1` + `e`.
    assert_trailing_junk("1e");
    assert_trailing_junk("123abc");
    assert_trailing_junk("1.5e3abc");
    assert_trailing_junk("1e+");
    // A number abutting `::`/`+`/`,`/whitespace is fine — those aren't identifier
    // bytes, so no junk. `$` continues a parameter, not the number.
    assert_eq!(
        kinds("1::int"),
        vec![TokenKind::Number, TokenKind::Cast, TokenKind::Identifier]
    );
    assert_eq!(
        kinds("5+3"),
        vec![TokenKind::Number, TokenKind::Plus, TokenKind::Number]
    );
    assert_eq!(kinds("1$1"), vec![TokenKind::Number, TokenKind::Param(1)]);
}

/// A radix-prefixed integer literal decodes to its decimal value (`0x1F` → `31`).
#[test]
fn radix_integer_literals() {
    for (src, want) in [
        ("0x1F", "31"),
        ("0X1f", "31"),
        ("0o17", "15"),
        ("0O17", "15"),
        ("0b101", "5"),
        ("0B101", "5"),
        ("0x0", "0"),
        // `_` may separate digits (incl. right after the prefix).
        ("0x_1F", "31"),
        ("0x1_F", "31"),
        ("0b1_0_1", "5"),
        // u64::MAX fits (decodes to numeric downstream); u32-crossing stays exact.
        ("0xFFFFFFFF", "4294967295"),
        ("0xFFFFFFFFFFFFFFFF", "18446744073709551615"),
    ] {
        let tok = one(src);
        assert_eq!(tok.kind, TokenKind::Number, "{src:?}");
        assert_eq!(tok.text(src), want, "{src:?}");
    }
    // No digits after the prefix, or a bad trailing byte, is a loud error.
    for bad in [
        "0x", "0o", "0b", "0xG", "0x1FG", "0x1F_", "0b2", "0o8", "0x__1",
    ] {
        assert!(tokenize(bad).is_err(), "{bad:?} should be a lex error");
    }
}

/// Assert `src` fails to lex with a "trailing junk after numeric literal" message.
fn assert_trailing_junk(src: &str) {
    let err = tokenize(src).unwrap_err();
    assert_eq!(client_sqlstate(&err, src), "42601", "{src:?} sqlstate");
    assert!(err.message.contains("trailing junk"), "{src:?} -> {err:?}");
}

#[test]
fn digit_group_underscores() {
    // A `_` between digits (in any of the three parts) is a separator: the token is
    // still one `Number`, and `text()` returns the underscore-stripped value.
    for (src, want) in [
        ("1_000", "1000"),
        ("1_000_000", "1000000"),
        ("1_000.5", "1000.5"),
        ("1_0.5e1_0", "10.5e10"),
        ("1.000_5", "1.0005"),
    ] {
        let tok = one(src);
        assert_eq!(tok.kind, TokenKind::Number, "{src:?}");
        assert_eq!(tok.text(src), want, "{src:?}");
    }
    // A plain number with no underscore keeps `decoded == None` (zero-copy).
    assert!(one("1000").decoded.is_none());
    // Leading `_` is an identifier, never a number.
    assert_eq!(one("_1000").kind, TokenKind::Identifier);
    // A doubled / trailing / decimal-adjacent `_` stops the number scan, leaving an
    // identifier byte glued to the literal — trailing junk, a loud error (Postgres
    // rejects all of these outright).
    for junk in ["1__000", "1000_", "1_.5", "1._5"] {
        assert!(tokenize(junk).is_err(), "{junk:?} should be a lex error");
    }
}

#[test]
fn plain_strings_and_doubled_quotes() {
    assert_eq!(value("'abc'"), "abc");
    assert_eq!(value("''"), "");
    assert_eq!(value("'it''s'"), "it's");
    assert_eq!(value("''''"), "'");
    // Under standard_conforming_strings a backslash is literal (hex-bytea input).
    assert_eq!(value("'\\x41'"), "\\x41");
    // A plain string with no transform stays zero-copy (no owned decode).
    assert!(one("'abc'").decoded.is_none());
    assert!(one("'it''s'").decoded.is_some());
}

#[test]
fn escape_strings() {
    assert_eq!(value("E'a\\nb'"), "a\nb");
    assert_eq!(value("E'tab\\there'"), "tab\there");
    assert_eq!(value("E'a\\\\b'"), "a\\b");
    assert_eq!(value("E'\\x41'"), "A");
    assert_eq!(value("E'\\101'"), "A"); // octal
    assert_eq!(value("E'\\u0041'"), "A");
    assert_eq!(value("E'\\v'"), "\u{000B}"); // the vertical tab, `unescape_single_char`'s sixth arm
    assert_eq!(value("E'it''s'"), "it's"); // `''` still works in E-strings
    assert_eq!(value("e'lower'"), "lower"); // case-insensitive E prefix
    assert_eq!(one("E'x'").kind, TokenKind::String);
}

#[test]
fn a_byte_escape_contributes_a_byte_and_not_a_code_point() {
    // `{xehexesc}` / `{xeoctesc}` append one byte, so a run of them that spells a UTF-8
    // character *is* that character. Read as code points these were two and three
    // characters. The octal arm truncates to eight bits on the way.
    for (src, want) in [
        ("E'\\xC3\\xA9'", "\u{00E9}"),
        ("E'\\303\\251'", "\u{00E9}"),
        ("E'\\xE2\\x82\\xAC'", "\u{20AC}"),
        ("E'\\x41\\xC3\\xA9\\x42'", "A\u{00E9}B"),
        ("E'\\x7F'", "\u{007F}"),
        ("E'\\177'", "\u{007F}"),
    ] {
        assert_eq!(value(src), want, "{src:?}");
    }
    // A backslash before a non-ASCII character keeps the whole character: 18.6 reads one
    // byte through `unescape_single_char` and the rest as ordinary content.
    assert_eq!(value("E'\\\u{00E9}'"), "\u{00E9}");
    assert_eq!(value("E'\\\u{1F4A9}'"), "\u{1F4A9}");
}

#[test]
fn a_bare_hex_escape_is_the_letter_x() {
    // `{xehexesc}` needs a hex digit; without one the shorter `{xeescape}` matches and
    // `unescape_single_char('x')` returns the letter. These were an over-rejection.
    for (src, want) in [
        ("E'\\x'", "x"),
        ("E'\\xZZ'", "xZZ"),
        ("E'\\xG'", "xG"),
        ("E'a\\x'", "ax"),
    ] {
        assert_eq!(value(src), want, "{src:?}");
    }
}

#[test]
fn an_invalid_encoding_names_the_bytes_from_the_failure() {
    // `report_invalid_encoding` prints `min(pg_utf_mblen(lead), remaining)` bytes from
    // the failure offset, and the scan that finds that offset runs once over the
    // finished literal, left to right — so an earlier bad byte outranks a later NUL.
    for (src, want) in [
        ("E'\\xE9'", "0xe9"),
        ("E'a\\xE9b'", "0xe9 0x62"),
        ("E'\\xE9\\xE9\\xE9\\xE9'", "0xe9 0xe9 0xe9"),
        ("E'\\xE9\\x00'", "0xe9 0x00"),
        ("E'\\x00\\xE9'", "0x00"),
        ("E'\\xC3\\x41'", "0xc3 0x41"),
        ("E'\\xC0\\x80'", "0xc0 0x80"),
        ("E'\\xED\\xA0\\x80'", "0xed 0xa0 0x80"),
        ("E'\\xF5\\x80\\x80\\x80'", "0xf5 0x80 0x80 0x80"),
        ("E'\\777'", "0xff"),
        ("E'\\400'", "0x00"),
    ] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "22021", "{src:?} -> {err:?}");
        assert_eq!(
            err.message,
            format!("invalid byte sequence for encoding \"UTF8\": {want}"),
            "{src:?}"
        );
    }
}

#[test]
fn surrogate_pairs_combine_across_both_spellings_in_both_decoders() {
    // Two adjacent escapes spelling a UTF-16 pair are one character, and which spelling
    // each half takes does not matter.
    for src in [
        "E'\\uD83D\\uDCA9'",
        "E'\\U0000D83D\\U0000DCA9'",
        "E'\\U0000D83D\\uDCA9'",
        "E'\\uD83D\\U0000DCA9'",
        "U&'\\D83D\\DCA9'",
        "U&'\\D83D\\+00DCA9'",
        "U&'\\+00D83D\\DCA9'",
        "U&'\\+00D83D\\+00DCA9'",
    ] {
        assert_eq!(value(src), "\u{1F4A9}", "{src:?}");
    }
}

#[test]
fn an_unfinished_surrogate_pair_is_a_syntax_error() {
    // A half with nothing to complete it is refused in both decoders, either way round.
    // The spellings with something *between* the halves are the rule's second part: a
    // pending half is refused where the next token is read, so a later second half does
    // not pair across whatever sat in between — without that, each of those would be an
    // accepted two-character string.
    for src in [
        "E'\\uD83D'",
        "E'\\uDCA9'",
        "E'\\uD83Dx'",
        "E'\\uD83D\\uD83D'",
        "E'\\uD83D\\x41'",
        "E'\\uD83D'''",
        "E'\\uD83Dx\\uDCA9'",
        "E'\\uD83D\\x41\\uDCA9'",
        "E'\\uD83D''\\uDCA9'",
        "U&'\\D83Dx\\DCA9'",
        "U&'\\D83D\\\\\\DCA9'",
        "U&'\\D83D''\\DCA9'",
        "U&'\\D83D'",
        "U&'\\DCA9'",
        "U&'\\D83D\\\\'",
        "U&'\\D83Dx'",
        "U&'\\D83D\\D83D'",
    ] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "42601", "{src:?} -> {err:?}");
        assert_eq!(err.message, "invalid Unicode surrogate pair", "{src:?}");
    }
}

#[test]
fn a_short_unicode_escape_splits_by_decoder() {
    // 18.6 disagrees with itself: `{xeunicodefail}` raises `22025` inside an `E'…'`
    // where the grammar's `U&` unescaper raises a plain `42601` for the same
    // malformation. Both sides are reproduced rather than unified.
    for src in [
        "E'\\u'",
        "E'\\u0'",
        "E'\\u00'",
        "E'\\u000'",
        "E'\\U'",
        "E'\\U0000000'",
        "E'\\uD83D\\u12'",
        "E'\\uD83D\\U12'",
    ] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "22025", "{src:?} -> {err:?}");
        assert_eq!(err.message, "invalid Unicode escape", "{src:?}");
    }
    for src in ["U&'\\00'", "U&'\\+00'", "U&'\\zz'", "U&'\\+zzzzzz'"] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "42601", "{src:?} -> {err:?}");
        assert_eq!(err.message, "invalid Unicode escape", "{src:?}");
    }
}

#[test]
fn an_escape_value_outside_the_code_point_range_is_a_syntax_error() {
    // `is_valid_unicode_codepoint` is `0 < c <= 0x10FFFF`, in both decoders.
    for src in [
        "E'\\U00110000'",
        "E'\\U00000000'",
        "E'\\u0000'",
        "U&'\\0000'",
        "U&'\\+110000'",
    ] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "42601", "{src:?} -> {err:?}");
        assert_eq!(err.message, "invalid Unicode escape value", "{src:?}");
    }
}

#[test]
fn the_value_check_and_the_pairing_check_run_in_each_decoder_s_own_order() {
    // `str_udeescape` runs `check_unicode_value` before it looks at the saved half, so a
    // pending first surrogate followed by an out-of-range value is the *value*'s error;
    // the `E'…'` scanner has no such check in the `xeu` state, so the same input is the
    // pairing's. Both codes are `42601`, so only the message separates the two orders —
    // and reversing either one is invisible to a fixture.
    for (src, want) in [
        ("E'\\uD83D\\u0000'", "invalid Unicode surrogate pair"),
        ("E'\\uD83D\\U00110000'", "invalid Unicode surrogate pair"),
        ("U&'\\D83D\\0000'", "invalid Unicode escape value"),
        ("U&'\\D83D\\+110000'", "invalid Unicode escape value"),
    ] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "42601", "{src:?} -> {err:?}");
        assert_eq!(err.message, want, "{src:?}");
    }
}

#[test]
fn a_plain_string_and_a_quoted_identifier_read_no_backslash() {
    // The byte-unit rule is `decode_escape`'s alone. Under
    // `standard_conforming_strings` a plain `'…'` collapses `''` and nothing else, and a
    // `"…"` identifier collapses `""` and nothing else — neither reads a backslash, so
    // neither can assemble a byte or fail an encoding check.
    assert_eq!(value(r"'\xE9'"), r"\xE9");
    assert_eq!(value(r"'\0'"), r"\0");
    assert_eq!(value(r"'\uD83D'"), r"\uD83D");
    assert_eq!(value(r#""a\xE9b""#), r"a\xE9b");
    assert_eq!(value(r#""\uD83D""#), r"\uD83D");
}

#[test]
fn unicode_strings() {
    assert_eq!(value("U&'\\0041'"), "A");
    assert_eq!(value("U&'\\+01F4A9'"), "\u{1F4A9}");
    assert_eq!(value("U&'plain'"), "plain");
    assert_eq!(value("U&'\\\\'"), "\\");
}

#[test]
fn nul_byte_is_rejected() {
    // A NUL produced by an octal / hex escape is `22021`: the byte reaches the finished
    // literal and encoding validation refuses it there, distinct from an ordinary
    // lexical error. `an_invalid_encoding_names_the_bytes_from_the_failure` beside this
    // covers where in the literal that verdict is taken.
    for src in ["E'a\\0b'", "E'\\x00'", "E'\\000'", "E'\\00'"] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "22021", "{src:?} -> {err:?}");
    }
    // A NUL written as a *Unicode* escape is rejected earlier as an invalid Unicode
    // code point — a `42601` syntax error, matching Postgres's split.
    for src in ["E'\\u0000'", "E'\\U00000000'", "U&'\\0000'"] {
        let err = tokenize(src).unwrap_err();
        assert_eq!(client_sqlstate(&err, src), "42601", "{src:?} -> {err:?}");
    }
    // A non-NUL octal/hex escape still decodes fine (`\101` = 'A', `\x41` = 'A').
    assert_eq!(value("E'\\101'"), "A");
    assert_eq!(value("E'\\x41'"), "A");
    assert_eq!(value("E'a\\tb'"), "a\tb");
}

#[test]
fn dollar_quoted_strings() {
    assert_eq!(value("$$abc$$"), "abc");
    assert_eq!(value("$tag$body$tag$"), "body");
    // The body is verbatim — embedded quotes and `$` need no escaping.
    assert_eq!(value("$$a'b\"c$$"), "a'b\"c");
    assert_eq!(value("$q$a$b$q$"), "a$b");
    // A differently-tagged dollar-quote inside is just body text.
    assert_eq!(value("$outer$ x $inner$ y $outer$"), " x $inner$ y ");
}

#[test]
fn parameters() {
    assert_eq!(one("$1").kind, TokenKind::Param(1));
    assert_eq!(one("$42").kind, TokenKind::Param(42));
    assert_eq!(
        kinds("$1::text"),
        vec![TokenKind::Param(1), TokenKind::Cast, TokenKind::Identifier]
    );
}

#[test]
fn operators() {
    assert_eq!(
        kinds("a::b"),
        vec![
            TokenKind::Identifier,
            TokenKind::Cast,
            TokenKind::Identifier
        ]
    );
    let cases = [
        ("@>", TokenKind::Containment),
        ("<@", TokenKind::ContainedBy),
        ("->", TokenKind::Arrow),
        ("->>", TokenKind::ArrowText),
        ("||", TokenKind::Concat),
        ("=", TokenKind::Eq),
        ("<>", TokenKind::NotEq),
        ("!=", TokenKind::NotEq),
        ("<", TokenKind::Lt),
        ("<=", TokenKind::Le),
        (">", TokenKind::Gt),
        (">=", TokenKind::Ge),
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("*", TokenKind::Star),
        ("/", TokenKind::Slash),
        ("%", TokenKind::Percent),
        ("^", TokenKind::Caret),
        ("::", TokenKind::Cast),
        (":", TokenKind::Colon),
    ];
    for (src, expected) in cases {
        assert_eq!(one(src).kind, expected, "{src:?}");
    }
}

#[test]
fn punctuation() {
    assert_eq!(
        kinds("(),;[]"),
        vec![
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::Comma,
            TokenKind::Semicolon,
            TokenKind::LBracket,
            TokenKind::RBracket,
        ]
    );
}

/// A `--` comment ends at a carriage return as well as a line feed — `scan.l`'s
/// `non_newline` is `[^\n\r]` — so the tokens after a lone `\r` are read.
#[test]
fn a_line_comment_ends_at_a_carriage_return() {
    assert_eq!(
        kinds("SELECT 1 --c\r+ 1"),
        vec![
            kw(Keyword::Select),
            TokenKind::Number,
            TokenKind::Plus,
            TokenKind::Number
        ]
    );
    assert_eq!(kinds("-- c\rSELECT"), vec![kw(Keyword::Select)]);
    assert_eq!(kinds("-- c\r\nSELECT"), vec![kw(Keyword::Select)]);
    // An operator run cut by a comment resumes after the carriage return.
    assert_eq!(
        kinds("1+--c\r2"),
        vec![TokenKind::Number, TokenKind::Plus, TokenKind::Number]
    );
    // A `;` inside the comment is comment text; the one after the `\r` is a token.
    assert_eq!(
        kinds("1 --c;\r;"),
        vec![TokenKind::Number, TokenKind::Semicolon]
    );
}

#[test]
fn comments_and_whitespace_are_skipped() {
    assert_eq!(kinds("  \t\n SELECT  "), vec![kw(Keyword::Select)]);
    assert_eq!(kinds("-- a comment\nSELECT"), vec![kw(Keyword::Select)]);
    assert_eq!(kinds("SELECT -- trailing"), vec![kw(Keyword::Select)]);
    // Block comments nest.
    assert_eq!(kinds("/* a /* b */ c */ 1"), vec![TokenKind::Number]);
    assert_eq!(
        kinds("1 /* x */ + /* y */ 2"),
        vec![TokenKind::Number, TokenKind::Plus, TokenKind::Number]
    );
}

#[test]
fn empty_and_whitespace_only_input() {
    assert_eq!(kinds(""), vec![]);
    assert_eq!(kinds("   \n\t"), vec![]);
    assert_eq!(tokenize("").unwrap().len(), 1); // just Eof
}

#[test]
fn errors_are_loud_with_positions() {
    let cases = [
        ("'unterminated", "string"),
        ("\"unterminated", "quoted identifier"),
        ("$tag$ body", "dollar-quoted"),
        ("/* open", "comment"),
        // A byte outside `scan.l`'s `self` and `op_chars` sets — its `{other}` rule.
        ("a \\ b", "unexpected"),
        ("a { b", "unexpected"),
    ];
    for (src, needle) in cases {
        let err = tokenize(src).unwrap_err();
        assert!(err.message.contains(needle), "{src:?} -> {err:?}");
        assert!(err.position <= src.len(), "{src:?} position out of range");
    }
}

#[test]
fn spine_on_conflict_upsert_tokenizes() {
    let sql = "INSERT INTO role_grant (actor_id, role) VALUES ($1, $2) \
	           ON CONFLICT (actor_id, role, COALESCE(scope_id, '')) WHERE revoked_at IS NULL \
	           DO UPDATE SET role = EXCLUDED.role";
    let tokens = tokenize(sql).unwrap();
    // Sanity: the expression ON CONFLICT target (the spine's parser gap) lexes
    // cleanly — keywords, params, a function call, and a `WHERE` predicate.
    assert!(tokens.iter().any(|t| t.kind == kw(Keyword::Conflict)));
    assert!(tokens.iter().any(|t| t.kind == TokenKind::Param(1)));
    assert!(tokens.iter().any(|t| matches!(t.kind, TokenKind::Eof)));
    assert!(
        !tokens
            .iter()
            .any(|t| t.kind == TokenKind::Eof && t.start != sql.len())
    );
}

#[test]
fn spine_correlated_not_exists_tokenizes() {
    let sql = "SELECT 1 FROM fact f WHERE NOT EXISTS \
	           (SELECT 1 FROM cell c WHERE c.refs @> ARRAY[f.hash]::text[] AND c.deleted_at IS NULL)";
    let kinds = kinds(sql);
    assert!(kinds.contains(&kw(Keyword::Not)));
    assert!(kinds.contains(&TokenKind::Containment));
    assert!(kinds.contains(&kw(Keyword::Array)));
    assert!(kinds.contains(&TokenKind::Cast));
}

/// Assert the structural invariants that must hold for any successful tokenization.
fn assert_well_formed(source: &str, tokens: &[Token]) {
    let mut prev_end = 0;
    for (i, token) in tokens.iter().enumerate() {
        assert!(
            token.start >= prev_end,
            "disorder/overlap in {source:?}: {token:?}"
        );
        assert!(
            token.end <= source.len(),
            "out of bounds in {source:?}: {token:?}"
        );
        let is_last = i + 1 == tokens.len();
        if is_last {
            assert_eq!(token.kind, TokenKind::Eof);
            assert_eq!(token.start, token.end);
            assert_eq!(token.end, source.len(), "Eof must sit at end of input");
        } else {
            assert!(
                token.start < token.end,
                "empty non-Eof token in {source:?}: {token:?}"
            );
            // `text` must produce a valid slice (no panic / bad char boundary).
            let _ = token.text(source);
        }
        prev_end = token.end;
    }
    assert!(matches!(
        tokens.last().map(|t| &t.kind),
        Some(TokenKind::Eof)
    ));
}

#[test]
fn fuzz_never_panics_and_spans_are_well_formed() {
    use pglet_testing::rng::Xorshift;

    // A byte pool biased toward lexically significant ASCII so the Ok path is
    // exercised, not just the early-error path. All bytes are ASCII → valid UTF-8.
    const POOL: &[u8] = b"abcXY_09 \t\n'\"$\\.,;:()[]<>=@|+-*/%^&ELU euntsiv0123";
    let mut rng = Xorshift::new(0x5151_2026_2026_5151);

    for _ in 0..50_000 {
        let len = rng.below(48) as usize;
        let source: String = (0..len)
            .map(|_| POOL[rng.below(POOL.len() as u64) as usize] as char)
            .collect();
        // Drive the lexer directly so a mid-stream error is also exercised.
        let mut lexer = Lexer::new(&source);
        let mut tokens = Vec::new();
        loop {
            match lexer.next_token() {
                Ok(token) => {
                    let done = matches!(token.kind, TokenKind::Eof);
                    tokens.push(token);
                    if done {
                        assert_well_formed(&source, &tokens);
                        break;
                    }
                }
                // An error is acceptable for arbitrary input; the contract is no panic.
                Err(err) => {
                    assert!(err.position <= source.len());
                    break;
                }
            }
        }
    }
}

#[test]
fn fuzz_valid_token_soup_round_trips() {
    use pglet_testing::rng::Xorshift;

    const PIECES: &[&str] = &[
        "SELECT", "from", "where", "x", "\"Q\"", "123", "1.5", "'str'", "'a''b'", "E'e\\n'",
        "$$d$$", "$1", "::", "@>", "<@", "->>", "||", "<=", "<>", "(", ")", ",", ".", "*",
        "COALESCE", "ARRAY", "-- c\n", "/* b */",
    ];
    let mut rng = Xorshift::new(0xD00D_2026);
    for _ in 0..10_000 {
        let count = rng.below(12) as usize;
        let mut source = String::new();
        for _ in 0..count {
            source.push_str(PIECES[rng.below(PIECES.len() as u64) as usize]);
            source.push(' ');
        }
        // A space-separated join of valid pieces always tokenizes.
        let tokens = tokenize(&source).unwrap();
        assert_well_formed(&source, &tokens);
    }
}

#[test]
fn a_zero_length_delimited_identifier_is_a_scanner_error() {
    // `""` is not a name in any position on 18.6 — the **scanner** refuses it, so the
    // code is `42601` and not `42602 invalid_name` (measured, since `invalid_name` is
    // the plausible guess). Every one of these was measured on 18.6 as
    // `42601 zero-length delimited identifier at or near """"`; pglet accepted all but
    // the last, and two of the accepts wrote: `CREATE TABLE "" (a int)` registered a
    // relation with an empty name, and `SELECT 1 AS ""` shipped a nameless descriptor.
    for sql in [
        r#"SELECT "" FROM t"#,
        r#"SELECT 1 AS """#,
        r#"CREATE TABLE "" (a int)"#,
        r#"CREATE TABLE zz ("" int)"#,
        r#"SELECT t."" FROM t"#,
        r#"SELECT * FROM """#,
        r#"SELECT * FROM t AS """#,
        r#"SET search_path TO """#,
        r#"SET search_path TO public, """#,
        r#"SET "" = 1"#,
        r#"SHOW """#,
        r#"DEALLOCATE """#,
    ] {
        let err = tokenize(sql).expect_err(sql);
        assert_eq!(client_sqlstate(&err, sql), "42601", "{sql}");
        assert_eq!(
            err.message, r#"zero-length delimited identifier at or near """""#,
            "{sql}"
        );
    }
    // A **doubled** quote is a different token and stays legal: `""""` is the
    // one-character name `"`, and both engines answer `42703 column """ does not
    // exist` for it over a table without that column.
    assert_eq!(value(r#""""""#), "\"");
    assert_eq!(kinds(r#""""""#), vec![TokenKind::QuotedIdentifier]);
    // And a zero-length **string** is not an identifier at all — it is legal here and
    // on 18.6 (`SET search_path TO ''`).
    assert_eq!(kinds("''"), vec![TokenKind::String]);
    assert_eq!(value("''"), "");
}

/// `scan.l`'s operator rules, one case each, against what 18.6 answers for the same
/// text (measured on the oracle, `SELECT` of each spelling):
///
/// | rule | spelling | token(s) |
/// | --- | --- | --- |
/// | `self`, one character | `+ - * / % ^ < > =` | that character's own token |
/// | `self`, not an `op_chars` byte | `, ( ) [ ] . ; :` | its own token, never an operator |
/// | `typecast` | `::` | `Cast` — `:` is no `op_chars` byte, so no operator holds one |
/// | `less_equals` / `greater_equals` / `less_greater` / `not_equals` | `<= >= <> !=` | the rule's own token, not an `Op` (`scan.l` orders them before `{operator}`, and flex takes the earlier rule on a tie) |
/// | `equals_greater` | `=>` | its own token, which pglet's grammar takes nowhere (`SELECT 1 => 2` is `42601` on 18.6) |
/// | `{operator}`, maximal munch | `<=>` `!!=` `~~` | one `Op` of the whole run |
/// | `{operator}`, comment cut | `1-/*c*/2` `1+--c` | the operator stops where the comment opens |
/// | `{operator}`, trailing `+`/`-` trimmed | `=-` `*-` `<>-` `->-` | the `+`/`-` is left for the next token |
/// | `{operator}`, trailing `+`/`-` kept | `^-` `!=-` `#-` `?-` `\|-` | one `Op`, the run holding a character SQL operators do not use |
/// | one character left after a cut or a trim | `=` `<` | the `self` token, as the `self` rule would have given |
/// | two characters left after a trim | `<>-` → `<>` | the two-character rule's token |
#[test]
fn the_operator_rules_are_the_reference_scanners() {
    use TokenKind::{
        Arrow, Cast, Colon, Comma, Concat, Containment, Dot, Eq, FatArrow, Ge, Gt, LParen, Le, Lt,
        Minus, NotEq, Number, Op, Percent, Plus, RParen, Semicolon, Slash, Star,
    };
    let cases: &[(&str, &[TokenKind])] = &[
        // `self`, and the punctuation that is not an `op_chars` byte.
        ("+", &[Plus]),
        ("-", &[Minus]),
        ("*", &[Star]),
        ("/", &[Slash]),
        ("%", &[Percent]),
        ("^", &[TokenKind::Caret]),
        ("<", &[Lt]),
        (">", &[Gt]),
        ("=", &[Eq]),
        (",", &[Comma]),
        ("(", &[LParen]),
        (")", &[RParen]),
        (".", &[Dot]),
        (";", &[Semicolon]),
        (":", &[Colon]),
        ("::", &[Cast]),
        // The four two-character rules ahead of `{operator}`, and `equals_greater`.
        ("<=", &[Le]),
        (">=", &[Ge]),
        ("<>", &[NotEq]),
        ("!=", &[NotEq]),
        ("=>", &[FatArrow]),
        // Maximal munch: one operator of the whole `op_chars` run.
        ("<=>", &[Op]),
        ("!!=", &[Op]),
        ("~~", &[Op]),
        ("!~~*", &[Op]),
        ("^@", &[Op]),
        ("`", &[Op]),
        ("~", &[Op]),
        ("&", &[Op]),
        ("#", &[Op]),
        ("|", &[Op]),
        ("@?", &[Op]),
        ("#-", &[Op]),
        // The spellings pglet's grammar has a kind for are still one operator token.
        ("||", &[Concat]),
        ("@>", &[Containment]),
        ("->", &[Arrow]),
        ("->>", &[TokenKind::ArrowText]),
        ("#>>", &[TokenKind::HashArrowText]),
        ("?|", &[TokenKind::QuestionPipe]),
        // A comment opening inside the run cuts it there.
        ("1-/*c*/2", &[Number, Minus, Number]),
        ("1+--c\n2", &[Number, Plus, Number]),
        // The trailing `+`/`-` trim: no character outside SQL's own operators, so the
        // sign is left for the next token.
        ("1=-1", &[Number, Eq, Minus, Number]),
        ("1*-1", &[Number, Star, Minus, Number]),
        ("1<>-1", &[Number, NotEq, Minus, Number]),
        ("1<=-1", &[Number, Le, Minus, Number]),
        ("1>=-1", &[Number, Ge, Minus, Number]),
        ("1->-1", &[Number, Arrow, Minus, Number]),
        // Every trailing `+`/`-` goes, one token at a time: `+-+-` leaves `+`, then
        // `-+-` leaves `-`, then `+-` leaves `+` (`SELECT 1+-+-1` is `2` on 18.6).
        ("1+-+-1", &[Number, Plus, Minus, Plus, Minus, Number]),
        // …and kept, where the run holds one of `~!@#^&|`?%`.
        ("2^-1", &[Number, Op, Number]),
        ("1!=-1", &[Number, Op, Number]),
        ("1?-1", &[Number, Op, Number]),
        ("1|-1", &[Number, Op, Number]),
        ("1%-1", &[Number, Op, Number]),
    ];
    for (sql, expected) in cases {
        assert_eq!(&kinds(sql), expected, "{sql:?}");
    }
    // The operator's text is its name, which is what the parser resolves it by.
    for (sql, name) in [("~~", "~~"), ("!=-", "!=-"), ("2^-1", "^-"), ("#-", "#-")] {
        let operator = tokenize(sql)
            .unwrap()
            .into_iter()
            .find(|t| t.kind == Op)
            .unwrap_or_else(|| panic!("{sql}: no operator"));
        assert_eq!(operator.text(sql), name, "{sql}");
    }
    // Every `op_chars` byte lexes; none is a lexical error, as none is on 18.6.
    for byte in "~!@#^&|`?+-*/%<>=".chars() {
        let sql = format!("SELECT 1 {byte} 1");
        tokenize(&sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    // A byte outside both sets is `scan.l`'s `{other}` rule — a token no grammar rule
    // takes there, and a lexical error here.
    for byte in ['\\', '{', '}', '"'] {
        let sql = format!("SELECT 1 {byte} 1");
        let err = tokenize(&sql).expect_err(&sql);
        assert_eq!(client_sqlstate(&err, &sql), "42601", "{sql}");
    }
}

/// A radix-prefixed literal has no width cap: 18.6 hands one too large for `int4` on as
/// an `FCONST`, which parse analysis makes a `numeric` — measured, `pg_typeof(0x` and
/// 38 `F`s `)` is `numeric` there, and the value is the decimal below. pglet's
/// accumulator used to overflow and refuse the literal.
#[test]
fn a_radix_literal_past_any_integer_width_decodes_to_its_decimal() {
    let big = format!("0x{}", "F".repeat(38));
    assert_eq!(
        one(&big).text(&big),
        "5708990770823839524233143877797980545530986495"
    );
    assert_eq!(one("0xFF").text("0xFF"), "255");
    assert_eq!(one("0b1_0000").text("0b1_0000"), "16");
    assert_eq!(one("0o17").text("0o17"), "15");
    // 64 octal digits of 7 — past `u128` and then some.
    let wide = format!("0o{}", "7".repeat(64));
    assert_eq!(one(&wide).text(&wide).len(), 58);
}

/// The decimal spelling a radix literal decodes to is **exact at every width**, which
/// the limb accumulator is the thing that could get wrong: it consumes several source
/// digits a step and carries base-`10^9` limbs, so a mis-sized chunk multiplier or a
/// lost carry shows up as a wrong digit rather than as a failure. Checked against a
/// schoolbook reference — one source digit at a time, one decimal digit a place — over
/// every base and a spread of widths, plus the two shapes a limb boundary can fall on
/// (a value that is exactly a power of the limb base, and one a digit past it).
#[test]
fn a_radix_literal_decodes_exactly_at_every_width() {
    /// The obvious decode: multiply the running decimal digits by `base` and add, per
    /// source digit. Quadratic and unused by the engine — that is the point.
    fn schoolbook(digits: &str, base: u32) -> String {
        let mut places: Vec<u8> = Vec::new();
        for c in digits.chars() {
            let Some(d) = c.to_digit(base) else { continue };
            let mut carry = u64::from(d);
            for place in &mut places {
                let value = u64::from(*place) * u64::from(base) + carry;
                *place = u8::try_from(value % 10).unwrap_or(0);
                carry = value / 10;
            }
            while carry > 0 {
                places.push(u8::try_from(carry % 10).unwrap_or(0));
                carry /= 10;
            }
        }
        if places.is_empty() {
            return "0".to_string();
        }
        places.iter().rev().map(|d| char::from(b'0' + d)).collect()
    }
    // Widths around every chunk boundary (`Radix::chunk` takes 7 hex / 9 octal / 29
    // binary digits a step) and well past several limbs.
    for width in [1, 6, 7, 8, 9, 13, 14, 15, 28, 29, 30, 57, 100, 301, 1000] {
        for (prefix, base, digit) in [("0x", 16, 'D'), ("0o", 8, '5'), ("0b", 2, '1')] {
            let digits: String = std::iter::repeat_n(digit, width).collect();
            let sql = format!("{prefix}{digits}");
            assert_eq!(
                one(&sql).text(&sql),
                schoolbook(&digits, base),
                "{prefix} × {width}"
            );
            // A power of the base: one leading digit, then zeros — the carry-free shape.
            let power = format!("{prefix}1{}", "0".repeat(width));
            let expected = schoolbook(&format!("1{}", "0".repeat(width)), base);
            assert_eq!(one(&power).text(&power), expected, "{prefix} 1e{width}");
        }
    }
}

/// A literal whose digit count alone proves it past what `numeric`'s format can hold
/// carries **no decoded spelling**, so the quadratic decode is never run on it — and
/// `parser::expr`'s `number_literal` reads that through
/// [`super::is_undecoded_radix_literal`] and owes 18.6's `22003`.
///
/// The bound is the count at which `base^(s - 1)` reaches `2^435412`, the smallest
/// power of two with 131 073 decimal digits, over the literal's **significant** digits
/// `s`: 108 854 hex digits, 145 139 octal, 435 413 binary. One digit below each, the decode
/// still runs — that is the widest input the engine will ever convert, and the test is
/// also the **time** bound on the declined side: a schoolbook decode of the declined
/// literal took tens of seconds (measured at 58 s for 150 000 hex digits against 18.4's
/// 0.41 s), where declining is immediate.
///
/// The leading-zero half is the other direction of the same bound, and the one a
/// same-digit repetition cannot see: a leading zero adds no weight, so `0x` + 108 853
/// zeros + `1` is `1` on 18.6 and must decode here, at any run length — counting the
/// whole digit run made it `22003`, and `"1".repeat(n)` has no leading zero to catch it
/// with. Measured on 18.6 at each base's boundary, both sides.
#[test]
fn a_radix_literal_past_the_numeric_format_cap_is_not_decoded() {
    for (prefix, declined, decoded) in [
        ("0x", 108_854, 108_853),
        ("0o", 145_139, 145_138),
        ("0b", 435_413, 435_412),
    ] {
        // A leading-zero run of any length is weightless: the literal decodes, and its
        // value is the significant digits' — measured `1` on 18.6 at, below and above
        // each `declined` run length, where counting the whole run answered `22003`
        // over the wire (measured at `declined - 1`; a longer run only more so).
        for zeros in [declined - 1, declined, declined + 1, 3 * declined] {
            let padded = format!("{prefix}{}1", "0".repeat(zeros));
            let token = one(&padded);
            assert_eq!(token.text(&padded), "1", "{prefix} 0×{zeros} then 1");
        }
        // …including a run that is only zeros (18.6: `0`), and one holding the `_`
        // separators the skip has to read past.
        let zeros = format!("{prefix}{}", "0".repeat(declined));
        assert_eq!(one(&zeros).text(&zeros), "0", "{prefix} all zeros");
        let underscored = format!("{prefix}0_0_0_1");
        assert_eq!(
            one(&underscored).text(&underscored),
            "1",
            "{prefix} 0_0_0_1"
        );
        // The cap still fires behind a leading-zero run: the count judged is the
        // significant one, not the shortened one.
        let behind = format!("{prefix}{}{}", "0".repeat(1000), "1".repeat(declined));
        assert!(
            one(&behind).decoded.is_none(),
            "{prefix} 0×1000 then over-cap"
        );
        let over = format!("{prefix}{}", "1".repeat(declined));
        let token = one(&over);
        assert!(token.decoded.is_none(), "{prefix} × {declined} decoded");
        assert_eq!(token.text(&over), over, "{prefix} × {declined} text");
        assert!(
            super::is_undecoded_radix_literal(token.text(&over)),
            "{prefix} × {declined} not recognized"
        );
        let under = format!("{prefix}{}", "1".repeat(decoded));
        let token = one(&under);
        assert!(token.decoded.is_some(), "{prefix} × {decoded} not decoded");
        assert!(
            !super::is_undecoded_radix_literal(token.text(&under)),
            "{prefix} × {decoded} read as undecoded"
        );
    }
    // Nothing else produces a `Number` token this predicate claims: a decimal literal's
    // text never begins with a radix prefix (`reject_trailing_junk` refuses the letter).
    assert!(!super::is_undecoded_radix_literal(one("0").text("0")));
    assert!(!super::is_undecoded_radix_literal(one("0x1F").text("0x1F")));
}

/// `scan.l`'s `{operator}` rule refuses a name at `NAMEDATALEN` — `nchars >=
/// NAMEDATALEN` is `operator too long`, a **scanner** error, so it outranks the
/// deferred `42883` a name pglet implements nothing for owes. The length read is the
/// one left after the comment cut and the trailing-`+`/`-` trim, so a run that trims
/// back under the bound is not too long. Measured on 18.6: 63 `*` is `42883`, 64 is
/// `42601 operator too long`, and 63 `*` followed by `++` trims to 63 and is `42883`.
#[test]
fn an_operator_name_is_refused_at_namedatalen() {
    let stars = |n: usize| "*".repeat(n);
    for (name, ok) in [
        (stars(63), true),
        (stars(64), false),
        (stars(65), false),
        (format!("{}++", stars(63)), true),     // trims to 63
        (format!("{}++", stars(64)), false),    // trims to 64
        (format!("{}~", stars(63)), false),     // 64, and `~` blocks no trim
        (format!("{}/*c*/", stars(64)), false), // the comment cut leaves 64
        (format!("{}/*c*/", stars(63)), true),  // the comment cut leaves 63
    ] {
        let sql = format!("SELECT 1 {name} 1");
        let result = tokenize(&sql);
        if ok {
            assert!(result.is_ok(), "{sql} refused");
        } else {
            let err = result.expect_err(&sql);
            assert_eq!(err.message, "operator too long", "{sql}");
            assert_eq!(client_sqlstate(&err, &sql), "42601", "{sql}");
            // The message carries no part of the name, where 18.6's inlines all of it.
            assert!(!err.message.contains('*'), "{sql}");
        }
    }
}

/// No numeric literal's refusal names the literal — the property, not a reading of the
/// grammar.
///
/// `parser::expr`'s `number_literal` composes two halves: this lexer's `Number` token
/// and [`crate::types::PgNumeric::parse`] over its text. Of that parse's two verdicts
/// only `InvalidInputSyntax` names the value it was given, so a spelling reaching it is
/// a message as wide as the literal — which is what `1e2147483647` was, at 200 072 B
/// for a 200 000-digit exponent against 18.6's 97.
///
/// The bound is **what it enumerates**, not "no spelling can": every string of up to
/// five characters over `0 1 9 . e + - _` — three digits, the decimal point, a lowercase
/// exponent marker, both exponent signs and the digit separator, which is a *sample* of
/// what the decimal number grammar reads and no part at all of what the radix one does
/// (`scan_radix_number` reads the `x`/`o`/`b` prefix letters in either case — `0X10`,
/// `0O10` and `0B10` answer 16, 8 and 2 on both engines — and `a`–`f`/`A`–`F`), so no
/// radix spelling is among the 37 448 — goes through the real lexer, and the 2 523 of
/// those that come back as one `Number` token go through the real parse. Five characters
/// is what reaches the shapes that have bitten: `1e1_0` (the `_` is stripped into the
/// token's `decoded` text), `9.e1`, `.9e-1`, `1_0.9` — each of them inside that alphabet,
/// which is the standing trap in naming an example. It does **not** reach a spelling
/// whose exponent passes the reader's cap, which needs twelve; that one is a fixture
/// (`arithmetic/numeric_literal_over_exponent_cap_loud.sql`).
///
/// The radix half is covered outside this test, over the wire: all 41 370 spellings of up
/// to four characters over `0 1 9 . e E x o b f F _ + -` sent to the release server as
/// `SELECT <spelling>` name the spelling in exactly two verdicts — 5 604
/// `42703 column "…" does not exist`, each naming an identifier, and the bare `.`'s
/// `42601 … found '.'` — and in **no** `22P02` or `22003`, which are the two codes
/// `PgNumeric::parse`'s verdicts carry.
#[test]
fn no_number_literal_is_refused_by_a_message_that_names_it() {
    const ALPHABET: [char; 8] = ['0', '1', '9', '.', 'e', '+', '-', '_'];
    let mut numbers = 0usize;
    for len in 1..=5u32 {
        for code in 0..ALPHABET.len().pow(len) {
            let mut rest = code;
            let spelling: String = (0..len)
                .map(|_| {
                    let c = ALPHABET[rest % ALPHABET.len()];
                    rest /= ALPHABET.len();
                    c
                })
                .collect();
            let Ok(tokens) = tokenize(&spelling) else {
                continue;
            };
            if tokens.len() != 2 || tokens[0].kind != TokenKind::Number {
                continue;
            }
            numbers += 1;
            let text = tokens[0].text(&spelling);
            if let Err(err) = crate::types::PgNumeric::parse(text) {
                assert!(
                    !matches!(err, crate::Error::InvalidInputSyntax { .. }),
                    "{spelling:?} → {err}"
                );
            }
        }
    }
    assert_eq!(numbers, 2_523, "the enumeration's own size moved");
    // The spellings the doc above names as reached — pinned, an example outside the
    // alphabet or the length being the standing way a claim like it goes wrong.
    for named in ["1e1_0", "9.e1", ".9e-1", "1_0.9"] {
        assert!(
            named.len() <= 5 && named.chars().all(|c| ALPHABET.contains(&c)),
            "{named} is outside what this test enumerates"
        );
        assert_eq!(one(named).kind, TokenKind::Number, "{named}");
    }
}

/// The **cost** bound on the widest literal the engine will ever convert, which the cap
/// above bounds the *input* of and nothing bounds the per-digit work of.
///
/// Base conversion is quadratic on both engines; what keeps it usable is the constant,
/// and `Decimal`'s constant is the two factors an accumulator can lose without losing a
/// digit: it consumes `Radix::chunk`'s several source digits a step and carries base-10^9
/// limbs rather than single decimal digits. Reverting either — the per-digit schoolbook
/// this replaced — leaves every value it decodes **correct**, so the exactness test above
/// cannot see it; only the clock can. Measured in a debug build at the widest accepted
/// input of each base: 1.4 s each here against about 63× that per-digit, and 18.4's own
/// `set_var_from_non_decimal_integer_str` takes 0.41 s over the same hex literal. The
/// bound is wide enough that a loaded box does not fail it and far under the revert.
#[test]
fn the_widest_decode_stays_within_a_wall_clock_bound() {
    const BOUND: std::time::Duration = std::time::Duration::from_secs(20);
    for (prefix, digits) in [("0x", 108_853usize), ("0o", 145_138), ("0b", 435_412)] {
        let sql = format!("{prefix}{}", "1".repeat(digits));
        let start = std::time::Instant::now();
        let token = one(&sql);
        assert!(token.decoded.is_some(), "{prefix} × {digits} declined");
        let elapsed = start.elapsed();
        assert!(elapsed < BOUND, "{prefix} × {digits} took {elapsed:?}");
    }
}
