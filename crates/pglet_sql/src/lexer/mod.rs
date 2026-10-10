//! pglet's native, Postgres-only SQL lexer.
//!
//! A streaming, pull-based tokenizer (`Lexer::next_token`) following `tsv`'s
//! lexer patterns — zero-copy `{kind, start, end}` tokens, lazy escape decoding,
//! a `phf` keyword table, and an ASCII byte-loop scan — pared down to the SQL
//! surface (no comment preservation, no ASI, no parser-driven re-lexing, no
//! arenas). It skips whitespace and `--` / `/* */` comments and fails loud on
//! anything outside the recognized token set — which is `scan.l`'s: every `op_chars`
//! spelling is a token here too ([`TokenKind::Op`], the generic operator), so a lexical
//! error is one 18.6's scanner raises as well, and an operator pglet implements nothing
//! for is refused by the parser at the statement's analysis rather than here.
//!
//! The token stream the native [`crate::parser`] consumes — its Pratt expression
//! parser and statement grammar walk these tokens directly.

mod core;
mod escapes;
pub mod pg_quote;
pub mod token;

#[cfg(test)]
mod tests;

pub use core::Lexer;
pub use token::{ALL_KEYWORDS, Keyword, PgClass, Token, TokenKind};

/// Whether `byte` is whitespace to the lexer: 18.6's `scanner_isspace` set.
///
/// Space, `\t`, `\n`, `\r`, `\v`, `\f`. Every other byte, each byte of a Unicode
/// space included, is part of a token or a comment, so a decision about statement text
/// that skips or splits on whitespace asks this rather than `char::is_whitespace`.
#[must_use]
pub const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// `text` less the lexer's whitespace ([`is_space`]) at either end.
#[must_use]
pub fn trim_space(text: &str) -> &str {
    text.trim_matches(|c: char| u8::try_from(c).is_ok_and(is_space))
}

/// The first `max` tokens of `sql`, comments and whitespace already gone.
///
/// The shared front half of every **statement classifier** that runs ahead of the
/// parser — the aborted block's exit classifier and `DEALLOCATE`'s. Each answers
/// from the leading words alone, so it stops at end of input, at a lex error, or
/// once `max` tokens are read, and a large statement is never tokenized to build a
/// gate answer. A statement the lexer cannot tokenize at all yields the tokens it
/// did read: such a statement is `42601` from the parser anyway, so a classifier
/// that declines it loses nothing.
#[must_use]
pub fn lead_tokens(sql: &str, max: usize) -> Vec<Token> {
    let mut lexer = Lexer::new(sql);
    let mut tokens = Vec::with_capacity(max);
    while tokens.len() < max {
        match lexer.next_token() {
            Ok(token) if matches!(token.kind, TokenKind::Eof) => break,
            Ok(token) => tokens.push(token),
            Err(_) => break,
        }
    }
    tokens
}

/// Every token of `sql`, or the lexer's own error.
///
/// [`lead_tokens`]'s counterpart for a reader that **answers the statement** rather
/// than classifying it. The silent stop is right for a classifier — a statement the
/// lexer cannot tokenize is the parser's `42601` anyway, so declining it costs
/// nothing — and wrong for a reader that then reports success or a code of its own:
/// `SET search_path TO ""` stopped at the lex error, left the reader looking at
/// `SET search_path TO`, and answered that reader's code instead of the scanner's
/// `42601`. A reader that decides an answer must see the whole statement or the
/// reason it cannot.
///
/// There is no token cap. A cap here bought nothing measurable — the statement has
/// already been split by `parser::split_statements` — and it cost an accept the
/// reader could not see past, and then an over-refusal at the cap that the
/// `set_config` route (which reads a whole string) did not share.
///
/// # Errors
///
/// The first [`LexError`] the scan hits.
pub fn all_tokens(sql: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer::new(sql);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        if matches!(token.kind, TokenKind::Eof) {
            return Ok(tokens);
        }
        tokens.push(token);
    }
}

/// Whether `token` is an **unquoted** word spelling `word` (which must be lowercase
/// ASCII).
///
/// The classifiers above read words the lexer does not carry as keywords —
/// `WORK`, `DEALLOCATE`, `PREPARE` — so they arrive as `Identifier`s and
/// [`Token::name`] applies the parser's own ASCII fold, which is the width that
/// matters: a Unicode fold would claim a statement the parser does not accept.
/// `QuotedIdentifier` is excluded rather than folded, because a quoted word is a
/// *name* — `"deallocate" x` is `42601` on 18.6, not a `DEALLOCATE`.
#[must_use]
pub fn is_word(token: &Token, source: &str, word: &str) -> bool {
    matches!(token.kind, TokenKind::Identifier) && token.name(source) == word
}

/// Whether `text` — a [`TokenKind::Number`] token's text — is a radix-prefixed
/// literal's **verbatim source spelling** rather than the decimal spelling the scanner
/// decodes one into.
///
/// A radix literal always carries its decimal spelling as the token's `decoded`
/// payload, so its text reads as a decimal number and this is false. It is true of
/// exactly one token: the literal `Radix::decode` declines, whose digit count alone
/// proves it past what `numeric`'s format can hold — `parser::expr`'s `number_literal`
/// reads that as the `22003` it is rather than as a malformed number. Nothing else
/// produces a `Number` token whose text starts `0x`/`0o`/`0b`: the scanner's decimal
/// rule stops at the `0` and `reject_trailing_junk` refuses the letter after it.
#[must_use]
pub fn is_undecoded_radix_literal(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes.next() == Some(b'0')
        && matches!(bytes.next(), Some(b'x' | b'X' | b'o' | b'O' | b'b' | b'B'))
}

/// A lexing failure, carrying a human message, the byte position where it occurred,
/// and what kind of failure it was.
///
/// The byte position drives the eventual syntax-error caret; the parser maps this to a
/// [`crate::Error`] at the boundary (`stream::lex_error_to_error`). Most of the lexer's
/// failures are ordinary `42601`s whose whole content is the message it writes here;
/// the rest are [`LexErrorKind`]'s other arms, which own no sentence of their own —
/// they carry their *data* and read the sentence off [`pglet_core::Error`], so the
/// wording and the SQLSTATE are not written twice.
///
/// That pairing is the **constructors'** convention and not a guarantee of the type:
/// the fields are public, so a hand-built value can put any message beside any kind,
/// and `from_kind` handed a `Syntax` would produce an empty-message one. `syntax` and
/// the kind constructors are what keep it true.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct LexError {
    pub message: String,
    pub position: usize,
    pub kind: LexErrorKind,
}

/// Which failure a [`LexError`] is, and therefore which [`crate::Error`] the parser
/// boundary builds from it.
///
/// Every arm but `Syntax` names a `pglet_core::Error` variant one-to-one, and
/// `into_error` is the mapping the parser boundary applies — so the SQLSTATE and the
/// sentence a client sees are `pglet_core::Error`'s rather than a second table's.
#[derive(Debug, Clone)]
pub enum LexErrorKind {
    /// `42601` — the default for almost every lexing failure (unterminated literals,
    /// trailing junk, a malformed `U&'…'` escape, a character no token begins with),
    /// each one a text 18.6's scanner refuses too.
    Syntax,
    /// `22021` — a finished `E'…'` literal that is not valid in the server encoding.
    /// Carries the bytes 18.6's `report_invalid_encoding` prints; the position does
    /// **not** travel with it, because 18.6 attaches no `P` to this error.
    InvalidEncoding { bytes: Vec<u8> },
    /// `22025` — a `\u` / `\U` escape inside an `E'…'` with too few hex digits.
    InvalidUnicodeEscape,
}

impl LexErrorKind {
    /// The crate error this kind becomes, or `None` for `Syntax`, whose message and
    /// position live on the [`LexError`] rather than in a variant.
    #[must_use]
    pub fn into_error(self) -> Option<pglet_core::Error> {
        match self {
            Self::Syntax => None,
            Self::InvalidEncoding { bytes } => Some(pglet_core::Error::InvalidEncoding { bytes }),
            Self::InvalidUnicodeEscape => Some(pglet_core::Error::InvalidUnicodeEscape),
        }
    }
}

impl LexError {
    /// A `42601` (syntax) lexical error — the default for almost every lexing
    /// failure (unterminated literals, trailing junk, a character `scan.l`'s `{other}`
    /// rule would return and no `gram.y` rule takes).
    #[must_use]
    pub fn syntax(position: usize, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position,
            kind: LexErrorKind::Syntax,
        }
    }

    /// A `22021` for a finished `E'…'` literal the server encoding refuses. `bytes` is
    /// the slice `report_invalid_encoding` prints, already cut by the caller.
    #[must_use]
    pub fn invalid_encoding(position: usize, bytes: Vec<u8>) -> Self {
        Self::from_kind(position, LexErrorKind::InvalidEncoding { bytes })
    }

    /// A `22025` for a `\u` / `\U` escape inside an `E'…'` with too few hex digits.
    #[must_use]
    pub fn invalid_unicode_escape(position: usize) -> Self {
        Self::from_kind(position, LexErrorKind::InvalidUnicodeEscape)
    }

    /// Build from a kind that owns its sentence, taking the message from the crate
    /// error so the lexer never writes a second copy of it.
    fn from_kind(position: usize, kind: LexErrorKind) -> Self {
        let message = kind
            .clone()
            .into_error()
            .map_or_else(String::new, |e| e.to_string());
        Self {
            message,
            position,
            kind,
        }
    }
}

/// Tokenize the whole of `source` into a vector, ending with an `Eof` token.
///
/// A convenience over [`Lexer::next_token`] for tests and non-streaming callers;
/// the parser drives the lexer one token at a time.
///
/// # Errors
///
/// Returns the first [`LexError`] encountered.
pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let is_eof = matches!(token.kind, TokenKind::Eof);
        tokens.push(token);
        if is_eof {
            return Ok(tokens);
        }
    }
}

/// Whether `name` is a plain lowercase identifier that needs no quoting.
///
/// It matches `^[a-z_][a-z0-9_]*$` **and** [`tokenize`] reads it as a single
/// `Identifier` token (not a keyword). The lexer check is the source of truth for
/// keyword-ness, so a reserved word that happens to be all-lowercase still gets quoted.
///
/// It lives beside the lexer because that is the whole of its content — the question
/// is "does *this* lexer read this name bare", and any second answer to it drifts.
#[must_use]
pub fn is_safe_bare_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_lowercase());
    if !first_ok {
        return false;
    }
    if !chars.all(|c| c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    // Confirm the lexer classifies it as a bare identifier (not a keyword). A clean
    // tokenization of a single word yields `[Identifier, Eof]`.
    matches!(
        tokenize(name).as_deref(),
        Ok([first, eof])
            if matches!(first.kind, TokenKind::Identifier)
                && matches!(eof.kind, TokenKind::Eof)
    )
}

/// Render `name` for an **identifier position** in SQL text.
///
/// Bare when [`is_safe_bare_ident`] accepts it, else double-quoted with any internal
/// `"` doubled — pg_dump's `fmtId` posture of quoting only when needed.
///
/// Over-quoting is **free where the string is SQL text** — both engines accept the
/// quoted form in the identifier positions this predicate feeds — and is a **wrong
/// value** where the string is catalog text a client reads or a value it reads back,
/// because there the string *is* the answer: a column named `action` renders
/// `"action"` here against 18.6's bare `action` at `pg_indexes.indexdef`'s key columns
/// and at `pg_get_constraintdef`, and a **relation** so named does the same through
/// `::regclass`, which resolves a relation and never a column (measured; `EDGES.md`
/// row 85 (f) carries the members,
/// `docs/names.md` § A name written back out the per-caller costs). Under-quoting has
/// two failure modes instead: a reserved word lexes wrong, and a name whose stored
/// spelling is not what an unquoted parse of it would fold to names something else.
///
/// Its callers are not one kind of thing, which is why the predicate is shared rather
/// than local to any of them: SQL text a reload must accept, catalog text a client
/// reads, a name written *inside* a string literal, and a **value** a client reads back
/// and may feed to `::regclass` again. The call sites are the enumeration; a list here
/// goes stale without reddening anything, and the one this doc carried had — it named a
/// consumer with no call site.
#[must_use]
pub fn quote_ident(name: &str) -> String {
    if is_safe_bare_ident(name) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}
