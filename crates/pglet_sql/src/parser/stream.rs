//! A token cursor over the streaming lexer with bounded (2-token) lookahead.
//!
//! The lexer is pull-based (`Lexer::next_token`), so the recursive-descent +
//! Pratt parser needs a small buffer to look ahead without consuming. SQL is
//! LL-friendly — 1–2 tokens of lookahead suffice (no byte-scan hacks) — so the
//! window is a fixed two-slot array holding the
//! current and next token, kept full (padded with repeated `Eof`). A `LexError`
//! is surfaced as [`Error::Syntax`] (SQLSTATE `42601`) at this boundary, keeping
//! the hot lexer free of crate-level error context.

use crate::Error;
use crate::lexer::{Keyword, Lexer, Token, TokenKind};

/// A pull cursor over [`Lexer`] with a two-token lookahead window.
pub struct TokenStream<'a> {
    source: &'a str,
    lexer: Lexer<'a>,
    /// The current token (`[0]`) and the one after it (`[1]`); always full.
    window: [Token; 2],
}

impl<'a> TokenStream<'a> {
    /// Create a cursor over `source`, priming the lookahead window.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if the first or second token fails to lex.
    pub fn new(source: &'a str) -> Result<Self, Error> {
        let mut lexer = Lexer::new(source);
        let t0 = pull(&mut lexer, source)?;
        let t1 = pull(&mut lexer, source)?;
        Ok(Self {
            source,
            lexer,
            window: [t0, t1],
        })
    }

    /// A restore point for a **speculative parse**: the lexer cursor and the
    /// lookahead window as they stand right now.
    ///
    /// The grammar has one construct two tokens of lookahead cannot decide, and it
    /// is the reason this exists: a *multi-word* typed constant
    /// (`DOUBLE PRECISION '1.5'`, `CHARACTER VARYING 'ab'`,
    /// `TIMESTAMP WITH TIME ZONE '…'`, `NUMERIC(5,2) '1.5'`) is a type name of
    /// unbounded length followed by a string, and every one of its lead words is
    /// also a legal bare column name on 18.6 (`SELECT double FROM tl` resolves
    /// there and here). Postgres's LALR grammar resolves that by deferring the
    /// reduction; a recursive-descent parser resolves it by trying the type-name
    /// production and putting the cursor back when no string follows.
    ///
    /// Two shapes commit instead. The single-word form (`DATE '…'`) never reaches a
    /// checkpoint at all — an identifier whose *next* token is a string is a typed
    /// constant unconditionally. A **committed window** takes one and never restores
    /// from it: where the lead word and the `(` after it can begin nothing but a type
    /// name, a failure inside the speculation is the statement's answer, because a
    /// restore would demote it to whatever the function-call path says next. Measured
    /// on both engines, `SELECT float(1) + 1` is `42601` while bare `SELECT float`
    /// stays an ordinary column reference (`42703`). See
    /// [`Parser::try_typed_constant`](super::Parser::try_typed_constant) for the
    /// membership test, which today admits `float` alone.
    ///
    /// Cheap: the lexer is a borrowed source plus an offset, and the window is two
    /// tokens (each an owned `String` only when a literal needed decoding).
    pub fn checkpoint(&self) -> StreamCheckpoint<'a> {
        StreamCheckpoint {
            lexer: self.lexer.clone(),
            window: self.window.clone(),
        }
    }

    /// Put the cursor back where [`checkpoint`](Self::checkpoint) was taken,
    /// discarding everything the speculative parse consumed.
    pub fn restore(&mut self, checkpoint: StreamCheckpoint<'a>) {
        self.lexer = checkpoint.lexer;
        self.window = checkpoint.window;
    }

    /// The whole text this stream reads.
    #[inline]
    pub const fn source(&self) -> &'a str {
        self.source
    }

    /// The kind of the current token.
    #[inline]
    pub const fn peek_kind(&self) -> &TokenKind {
        &self.window[0].kind
    }

    /// The kind of the token one past the current.
    #[inline]
    pub const fn peek2_kind(&self) -> &TokenKind {
        &self.window[1].kind
    }

    /// Consume and return the current token, refilling the window.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if the incoming token fails to lex.
    pub fn advance(&mut self) -> Result<Token, Error> {
        let incoming = pull(&mut self.lexer, self.source)?;
        let next = std::mem::replace(&mut self.window[1], incoming);
        Ok(std::mem::replace(&mut self.window[0], next))
    }

    /// Consume the current token, discarding it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if the incoming token fails to lex.
    #[inline]
    pub fn bump(&mut self) -> Result<(), Error> {
        self.advance().map(|_| ())
    }

    /// Whether the current token has exactly `kind`.
    #[inline]
    pub fn at(&self, kind: &TokenKind) -> bool {
        &self.window[0].kind == kind
    }

    /// Whether the current token is keyword `kw`.
    #[inline]
    pub fn at_kw(&self, kw: Keyword) -> bool {
        matches!(self.window[0].kind, TokenKind::Keyword(k) if k == kw)
    }

    /// Whether the cursor is at end of input.
    #[inline]
    pub const fn at_eof(&self) -> bool {
        matches!(self.window[0].kind, TokenKind::Eof)
    }

    /// Consume the current token iff it has `kind`; returns whether it did.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if a consumed refill fails to lex.
    pub fn eat(&mut self, kind: &TokenKind) -> Result<bool, Error> {
        if self.at(kind) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Consume the current token iff it is keyword `kw`; returns whether it did.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if a consumed refill fails to lex.
    pub fn eat_kw(&mut self, kw: Keyword) -> Result<bool, Error> {
        if self.at_kw(kw) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Consume the current token, requiring it to have `kind`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if the current token is not `kind` (or a refill
    /// fails to lex).
    pub fn expect(&mut self, kind: &TokenKind) -> Result<Token, Error> {
        if self.at(kind) {
            self.advance()
        } else {
            Err(self.expected(&kind.to_string()))
        }
    }

    /// Consume the current token, requiring it to be keyword `kw`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`] if the current token is not `kw`.
    pub fn expect_kw(&mut self, kw: Keyword) -> Result<(), Error> {
        if self.at_kw(kw) {
            self.bump()
        } else {
            Err(self.expected(&format!("'{}'", kw.as_str())))
        }
    }

    /// The semantic text of `tok` (decoded payload, or the delimiter-stripped
    /// source slice). See [`Token::text`].
    ///
    /// The result may borrow from `tok` (its decoded buffer) or from the source,
    /// so it is tied to the token's lifetime (the source outlives it: `'a: 't`).
    #[inline]
    pub fn text<'t>(&self, tok: &'t Token) -> &'t str
    where
        'a: 't,
    {
        tok.text(self.source)
    }

    /// The text of `tok` **as a name** — downcased when unquoted, verbatim when
    /// `"…"`-quoted. See [`Token::name`]; every name-producing read in the parser
    /// goes through this rather than [`text`](Self::text).
    #[inline]
    pub fn name(&self, tok: &Token) -> String {
        tok.name(self.source)
    }

    /// The semantic text of the current token (its decoded payload or the
    /// delimiter-stripped source slice). For a keyword or bare identifier this is
    /// the verbatim source spelling — used for case-insensitive contextual matching.
    #[inline]
    pub fn peek_text(&self) -> &str {
        self.window[0].text(self.source)
    }

    /// The semantic text of the token one past the current — [`Self::peek_text`] one
    /// further on.
    #[inline]
    pub fn peek2_text(&self) -> &str {
        self.window[1].text(self.source)
    }

    /// The byte position of the current token (for error reporting).
    #[inline]
    pub const fn pos(&self) -> usize {
        self.window[0].start
    }

    /// 18.6's `P` for the current token: a **1-based Unicode codepoint** index.
    ///
    /// The position used to be spelled into the message as a 0-based *byte* offset,
    /// which was wrong twice over — 18.6's `M` never carries a position at all, and a
    /// client reading `.position` off its driver got nothing while a multibyte prefix
    /// made the number in the text disagree with the oracle's (`'ααααα'` → pglet 26,
    /// 18.6 22).
    #[inline]
    fn codepoint_pos(&self) -> u32 {
        pglet_core::codepoint_position(self.source, self.pos())
    }

    /// 18.6's `P` for `tok` — the same 1-based codepoint index
    /// [`codepoint_pos`](Self::codepoint_pos) computes for the *current* token.
    ///
    /// A reader that must point at a token it has already consumed needs this one:
    /// `opt_float`'s range refusal reports the number's own position
    /// (`parser_errposition(@2)`), and the number is read before it can be judged.
    #[inline]
    pub fn token_position(&self, tok: &Token) -> u32 {
        pglet_core::codepoint_position(self.source, tok.start)
    }

    /// A `42601` syntax error at the current position, naming what was expected
    /// and what was found.
    pub fn expected(&self, what: &str) -> Error {
        Error::syntax_at(
            format!("expected {what}, found {}", self.window[0].kind),
            self.codepoint_pos(),
        )
    }

    /// A `42601` syntax error at the current position with a free-form message.
    pub fn error(&self, message: &str) -> Error {
        Error::syntax_at(message, self.codepoint_pos())
    }

    /// A `42601` syntax error in Postgres's `syntax error at or near "<tok>"`
    /// shape, naming the current token verbatim. Used when a statement doesn't
    /// begin with a recognized statement keyword — a typo, garbage, or a
    /// misplaced clause word (`FROM 1`) — where Postgres reports `syntax error at
    /// or near "FROM"` rather than an "unsupported construct" error.
    ///
    /// The sentence is complete, so it does **not** go through [`Error::syntax`]'s
    /// prefix — that is what produced `syntax error: syntax error at or near "X"`.
    pub fn syntax_error_at_or_near(&self) -> Error {
        Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", self.peek_text()),
            self.codepoint_pos(),
        )
    }
}

/// A saved [`TokenStream`] position — see [`TokenStream::checkpoint`].
#[derive(Clone)]
pub struct StreamCheckpoint<'a> {
    lexer: Lexer<'a>,
    window: [Token; 2],
}

/// Pull the next token, mapping a [`crate::lexer::LexError`] to a crate [`Error`].
#[inline]
fn pull<'a>(lexer: &mut Lexer<'a>, source: &'a str) -> Result<Token, Error> {
    lexer
        .next_token()
        .map_err(|e| lex_error_to_error(e, source))
}

/// Map a [`crate::lexer::LexError`] to a crate [`Error`] at the lexer→parser
/// boundary. Ordinary lexical failures become [`Error::Syntax`] (`42601`) carrying
/// 18.6's `P`; every other kind names a crate variant one-to-one and is handed through
/// whole, keeping Postgres's SQLSTATE and its sentence.
/// Shared with `parser::split_statements` and with `catalog::commands`' session-command
/// readers, which drive the lexer directly — one mapping, so a lexical failure has one
/// SQLSTATE wherever it is raised.
///
/// The dropped `position` on those other kinds is deliberate: 18.6 sends no `P` with the
/// encoding error, and the lexer's offset is the *literal*'s start rather than the
/// escape's, so it would be an invented field at the `22025` too.
pub fn lex_error_to_error(e: crate::lexer::LexError, source: &str) -> Error {
    let crate::lexer::LexError {
        message,
        position,
        kind,
    } = e;
    kind.into_error().unwrap_or_else(|| {
        Error::syntax_at(message, pglet_core::codepoint_position(source, position))
    })
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn peek_advance_window() {
        let mut ts = TokenStream::new("select 1 + 2").unwrap();
        assert!(ts.at_kw(Keyword::Select));
        assert_eq!(*ts.peek2_kind(), TokenKind::Number);
        ts.bump().unwrap(); // select
        assert_eq!(*ts.peek_kind(), TokenKind::Number);
        assert_eq!(*ts.peek2_kind(), TokenKind::Plus);
        let one = ts.advance().unwrap();
        assert_eq!(ts.text(&one), "1");
        assert!(ts.at(&TokenKind::Plus));
    }

    #[test]
    fn expect_and_eat() {
        let mut ts = TokenStream::new("(a)").unwrap();
        ts.expect(&TokenKind::LParen).unwrap();
        assert_eq!(*ts.peek_kind(), TokenKind::Identifier);
        ts.bump().unwrap();
        assert!(ts.eat(&TokenKind::RParen).unwrap());
        assert!(ts.at_eof());
        // expecting the wrong kind is a 42601 syntax error
        let err = ts.expect(&TokenKind::Comma).unwrap_err();
        assert_eq!(err.sqlstate(), "42601");
    }

    #[test]
    fn eof_is_sticky() {
        let mut ts = TokenStream::new("").unwrap();
        assert!(ts.at_eof());
        ts.bump().unwrap();
        assert!(ts.at_eof());
        assert!(ts.at_eof());
    }
}
