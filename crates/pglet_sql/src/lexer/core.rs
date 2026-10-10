//! The streaming pull lexer: `next_token()` yields one token at a time, no token
//! vector. Scanning is byte-based — every structural delimiter (quotes, `$`,
//! operators, whitespace) is ASCII, and ASCII bytes never occur inside a UTF-8
//! multi-byte sequence, so byte slices always land on char boundaries.

use super::LexError;
use super::escapes;
use super::token::{Token, TokenKind, keyword_kind};

/// Which string-literal form is being scanned (sets delimiter widths + decoding).
#[derive(Clone, Copy)]
enum StringForm {
    /// `'…'` — only `''` collapses (backslash literal under standard strings).
    Plain,
    /// `E'…'` — C-style backslash escapes plus `''`.
    Escape,
    /// `U&'…'` — `\XXXX` / `\+XXXXXX` Unicode escapes plus `''`.
    Unicode,
}

impl StringForm {
    /// Byte offset of the opening quote relative to the token start.
    const fn prefix_len(self) -> usize {
        match self {
            Self::Plain => 0,
            Self::Escape => 1,  // E'
            Self::Unicode => 2, // U&'
        }
    }
}

/// A pull-based SQL lexer over borrowed source.
///
/// `Clone` is a **cursor copy**: the state is a borrowed source plus a byte offset,
/// so a clone is a second cursor over the same text at the same position and costs
/// nothing to make. `parser::stream`'s speculative-parse checkpoint is the one
/// consumer (see `TokenStream::checkpoint`).
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// Create a lexer positioned at the start of `source`.
    #[must_use]
    pub const fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
        }
    }

    /// The byte the cursor is on, if any.
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// The byte `n` ahead of the cursor, if any.
    #[inline]
    fn peek_at(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.pos + n).copied()
    }

    /// Pull the next token, skipping leading whitespace and comments. Returns an
    /// `Eof` token at end of input (repeatedly).
    ///
    /// # Errors
    ///
    /// Returns `LexError` on an unterminated string/dollar-quote/block comment, a
    /// malformed literal, or a character `scan.l`'s `{other}` rule would return.
    pub fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_trivia()?;
        let start = self.pos;
        let Some(b) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                start,
                end: start,
                decoded: None,
            });
        };
        match b {
            // `E'…'` / `U&'…'` must be detected before the identifier path.
            b'e' | b'E' if self.peek_at(1) == Some(b'\'') => {
                self.scan_string(start, StringForm::Escape)
            }
            b'u' | b'U' if self.peek_at(1) == Some(b'&') && self.peek_at(2) == Some(b'\'') => {
                self.scan_string(start, StringForm::Unicode)
            }
            b'\'' => self.scan_string(start, StringForm::Plain),
            b'"' => self.scan_quoted_identifier(start),
            b'$' => self.scan_dollar(start),
            b'0'..=b'9' => self.scan_number(start),
            b'.' if matches!(self.peek_at(1), Some(d) if d.is_ascii_digit()) => {
                self.scan_number(start)
            }
            b if is_ident_start(b) => Ok(self.scan_identifier(start)),
            _ => self.scan_operator(start, b),
        }
    }

    /// Skip whitespace, `--` line comments, and (nesting) `/* … */` block comments.
    ///
    /// A `--` comment ends at a line feed **or a carriage return**, as `scan.l`'s does
    /// (`comment ("--"{non_newline}*)`, `non_newline [^\n\r]`): a client that breaks
    /// lines with a bare `\r` has the SQL after the comment read, not swallowed. The
    /// statement splitter, the transaction-exit classifier and the parser all take their
    /// tokens from here, so they agree on where a comment ends.
    fn skip_trivia(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(b) if super::is_space(b) => self.pos += 1,
                Some(b'-') if self.peek_at(1) == Some(b'-') => {
                    self.pos += 2;
                    while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => self.skip_block_comment()?,
                _ => return Ok(()),
            }
        }
    }

    /// Skip a `/* … */` block comment, which nests (per Postgres).
    fn skip_block_comment(&mut self) -> Result<(), LexError> {
        let start = self.pos;
        self.pos += 2;
        let mut depth = 1u32;
        while depth > 0 {
            match self.peek() {
                None => return Err(lex_error(start, "unterminated /* comment")),
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    depth += 1;
                    self.pos += 2;
                }
                Some(b'*') if self.peek_at(1) == Some(b'/') => {
                    depth -= 1;
                    self.pos += 2;
                }
                Some(_) => self.pos += 1,
            }
        }
        Ok(())
    }

    /// Scan a bare identifier or keyword (first byte already known an ident start).
    fn scan_identifier(&mut self, start: usize) -> Token {
        self.pos += 1;
        while matches!(self.peek(), Some(b) if is_ident_cont(b)) {
            self.pos += 1;
        }
        let text = &self.source[start..self.pos];
        let kind = keyword_kind(text).map_or(TokenKind::Identifier, TokenKind::Keyword);
        Token {
            kind,
            start,
            end: self.pos,
            decoded: None,
        }
    }

    /// Scan a `"…"` quoted identifier, collapsing `""` → `"` when present.
    ///
    /// A **zero-length** one is refused here, which is where Postgres refuses it: the
    /// scanner, not any statement's grammar. `""` is not a name in any position —
    /// measured on 18.6, every one of `SELECT "" FROM t`, `SELECT 1 AS ""`,
    /// `CREATE TABLE "" (a int)`, `CREATE TABLE zz ("" int)`, `SELECT t."" FROM t`,
    /// `SELECT * FROM ""`, `SELECT * FROM t AS ""`, `SET "" = 1`, `SHOW ""`,
    /// `SET search_path TO ""` and `DEALLOCATE ""` is
    /// `42601 zero-length delimited identifier at or near """"`. pglet used to accept
    /// it everywhere but `DEALLOCATE`, which hand-rolled the same check, and the
    /// accepts were not harmless: `CREATE TABLE "" (a int)` registered a **relation
    /// with an empty name** in the catalog, and `SELECT 1 AS ""` shipped a column
    /// whose descriptor names nothing.
    ///
    /// `42601` and not `42602 invalid_name` — measured, since `invalid_name` is the
    /// plausible guess; the scanner raises it as an ordinary syntax error before any
    /// name is looked at. A doubled quote is a different thing and stays legal:
    /// `""""` is the one-character name `"`.
    ///
    /// This is the identifier position only. A zero-length **string** is legal
    /// (`SET search_path TO ''`), and so is an empty quoted part *inside* a GUC list
    /// value — `set_config('search_path', '""', false)` answers `""` on 18.6 — which
    /// is why `catalog::commands`' `search_path` decider still admits an empty
    /// element from its string reader.
    fn scan_quoted_identifier(&mut self, start: usize) -> Result<Token, LexError> {
        self.pos += 1; // opening quote
        let mut doubled = false;
        loop {
            match self.peek() {
                None => return Err(lex_error(start, "unterminated quoted identifier")),
                Some(b'"') if self.peek_at(1) == Some(b'"') => {
                    doubled = true;
                    self.pos += 2;
                }
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => self.pos += 1,
            }
        }
        // `""` — the delimiters and nothing between them. A doubled quote cannot reach
        // this: `""""` leaves `doubled` set and a two-byte span.
        if self.pos - start == 2 {
            return Err(lex_error(
                start,
                "zero-length delimited identifier at or near \"\"\"\"",
            ));
        }
        let decoded =
            doubled.then(|| escapes::decode_quoted_ident(&self.source[start + 1..self.pos - 1]));
        Ok(Token {
            kind: TokenKind::QuotedIdentifier,
            start,
            end: self.pos,
            decoded,
        })
    }

    /// Scan a `'…'` / `E'…'` / `U&'…'` string literal.
    fn scan_string(&mut self, start: usize, form: StringForm) -> Result<Token, LexError> {
        let quote = start + form.prefix_len();
        self.pos = quote + 1; // first content byte
        let escapes_quote = matches!(form, StringForm::Escape);
        let mut decode = false;
        loop {
            match self.peek() {
                None => return Err(lex_error(start, "unterminated string literal")),
                Some(b'\'') if self.peek_at(1) == Some(b'\'') => {
                    decode = true;
                    self.pos += 2; // doubled quote
                }
                Some(b'\'') => {
                    self.pos += 1; // closing quote
                    break;
                }
                Some(b'\\') => {
                    decode = true;
                    // In an E-string, `\'` does not close the literal: consume the
                    // backslash and its escaped byte together.
                    self.pos += if escapes_quote { 2 } else { 1 };
                }
                Some(_) => self.pos += 1,
            }
        }
        let content = &self.source[quote + 1..self.pos - 1];
        // `E'…'` / `U&'…'` always decode (their value never equals a 1-char strip);
        // a plain `'…'` only decodes when it carried a `''`.
        let decoded = match form {
            StringForm::Plain => decode.then(|| escapes::decode_plain(content)),
            StringForm::Escape => Some(escapes::decode_escape(content, start)?),
            StringForm::Unicode => Some(escapes::decode_unicode(content, start)?),
        };
        Ok(Token {
            kind: TokenKind::String,
            start,
            end: self.pos,
            decoded,
        })
    }

    /// Scan `$` — either a `$N` positional parameter or a `$tag$…$tag$` string.
    fn scan_dollar(&mut self, start: usize) -> Result<Token, LexError> {
        match self.peek_at(1) {
            Some(d) if d.is_ascii_digit() => {
                self.pos += 1;
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    self.pos += 1;
                }
                let n: u32 = self.source[start + 1..self.pos]
                    .parse()
                    .map_err(|_| lex_error(start, "parameter index out of range"))?;
                Ok(Token {
                    kind: TokenKind::Param(n),
                    start,
                    end: self.pos,
                    decoded: None,
                })
            }
            Some(b) if b == b'$' || is_ident_start(b) => self.scan_dollar_quote(start),
            _ => Err(lex_error(start, "unexpected '$'")),
        }
    }

    /// Scan a `$tag$…$tag$` dollar-quoted string (the tag may be empty: `$$…$$`).
    ///
    /// The body is verbatim (no escapes), so it is always materialized into
    /// `decoded` — these are rare, so the allocation is not worth avoiding.
    fn scan_dollar_quote(&mut self, start: usize) -> Result<Token, LexError> {
        // Read the opening tag, terminated by the second `$`.
        let mut i = start + 1;
        loop {
            match self.bytes.get(i).copied() {
                Some(b'$') => break,
                Some(b) if is_ident_cont(b) => i += 1,
                _ => return Err(lex_error(start, "invalid dollar-quote tag")),
            }
        }
        let open = &self.source[start..=i]; // "$tag$"
        let body_start = i + 1;
        let rest = &self.source[body_start..];
        match rest.find(open) {
            Some(rel) => {
                let body_end = body_start + rel;
                self.pos = body_end + open.len();
                let body = self.source[body_start..body_end].to_string();
                Ok(Token {
                    kind: TokenKind::String,
                    start,
                    end: self.pos,
                    decoded: Some(body),
                })
            }
            None => Err(lex_error(start, "unterminated dollar-quoted string")),
        }
    }

    /// Scan a numeric literal: a radix-prefixed integer (`0x`/`0o`/`0b`, Postgres
    /// 16+), or a decimal integer/fraction/exponent. A `_` digit-group separator
    /// (Postgres 16+) is consumed only *between* two digits; when present the token
    /// carries a `decoded` copy with the underscores stripped, so downstream parsing
    /// sees a clean numeric string.
    ///
    /// An identifier byte glued directly to the end of the literal is **trailing
    /// junk** (`123abc`, `1e`, `1000_`, `0x1FG`) — a loud `42601` lexical error,
    /// matching Postgres's `{integer|numeric|real}_junk` scanner rules.
    fn scan_number(&mut self, start: usize) -> Result<Token, LexError> {
        // Radix-prefixed integer literal: `0x…` / `0o…` / `0b…`.
        if self.peek() == Some(b'0') {
            let radix = match self.peek_at(1) {
                Some(b'x' | b'X') => Some(Radix::Hex),
                Some(b'o' | b'O') => Some(Radix::Oct),
                Some(b'b' | b'B') => Some(Radix::Bin),
                _ => None,
            };
            if let Some(radix) = radix {
                return self.scan_radix_number(start, radix);
            }
        }
        let mut has_underscore = self.scan_digit_run();
        // Fractional part (Postgres allows a trailing dot: `5.`).
        if self.peek() == Some(b'.') {
            self.pos += 1;
            has_underscore |= self.scan_digit_run();
        }
        // Exponent — only if a digit (optionally signed) follows the `e`/`E`,
        // otherwise the `e` belongs to a following token (and, being an identifier
        // byte, is caught as trailing junk below: `1e` is an error, not `1` + `e`).
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mut j = self.pos + 1;
            if matches!(self.bytes.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if matches!(self.bytes.get(j), Some(d) if d.is_ascii_digit()) {
                // Position at the first exponent digit; `scan_digit_run` then takes
                // the digit run (with any interior `_`). The `e`/sign stay in the
                // token span but are dropped from `decoded` only if it is built.
                self.pos = j;
                has_underscore |= self.scan_digit_run();
            }
        }
        self.reject_trailing_junk(start)?;
        // Strip the underscores once, only when the literal actually carried one.
        let decoded = has_underscore.then(|| {
            self.source[start..self.pos]
                .bytes()
                .filter(|&b| b != b'_')
                .map(char::from)
                .collect()
        });
        Ok(Token {
            kind: TokenKind::Number,
            start,
            end: self.pos,
            decoded,
        })
    }

    /// Scan a radix-prefixed integer literal (`0x1F`, `0o17`, `0b101`; Postgres
    /// 16+). The `0x`/`0o`/`0b` prefix is already known; the grammar is
    /// `0[xob](_?digit)+` — a `_` separator may precede any digit (`0x_1F`,
    /// `0x1_F`), but not double up or trail. The value is decoded to its decimal
    /// spelling in `decoded`, so it funnels through the same `Int`/`Numeric` literal
    /// typing as a decimal integer (int4 → int8 → numeric by magnitude).
    ///
    /// **There is no width cap on the literal**: 18.6's `process_integer_literal` hands a
    /// radix literal too large for `int4` on as an `FCONST`, which parse analysis makes a
    /// `numeric` — `SELECT pg_typeof(0x` + 38 `F`s + `)` is `numeric` there, measured — so
    /// the decimal spelling is accumulated in base-`10^9` limbs ([`Decimal`]) rather than
    /// in an integer accumulator, whose overflow would refuse a literal 18.6 answers.
    ///
    /// **There is a cap on the decode**, [`Radix::decode`]'s: base conversion is
    /// quadratic in the digit count on both engines, and 18.6 stops its own at the point
    /// no `numeric` can hold the value. A literal whose count of **significant** digits
    /// alone proves it past that point carries **no decoded spelling** — `decoded` is
    /// `None`, so the token's text is its verbatim source — and `parser::expr`'s
    /// `number_literal` reads that through [`crate::lexer::is_undecoded_radix_literal`]
    /// and owes 18.6's
    /// `22003 value overflows numeric format`.
    fn scan_radix_number(&mut self, start: usize, radix: Radix) -> Result<Token, LexError> {
        self.pos += 2; // consume the `0x`/`0o`/`0b` prefix
        let body_start = self.pos;
        let mut digits = 0usize;
        loop {
            // An optional single `_` may precede a digit (but not a non-digit).
            if self.peek() == Some(b'_')
                && matches!(self.peek_at(1), Some(n) if radix.digit(n).is_some())
            {
                self.pos += 1;
            }
            if self.peek().and_then(|b| radix.digit(b)).is_none() {
                break;
            }
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(lex_error(start, radix.empty_message()));
        }
        self.reject_trailing_junk(start)?;
        Ok(Token {
            kind: TokenKind::Number,
            start,
            end: self.pos,
            decoded: radix.decode(&self.source[body_start..self.pos], digits),
        })
    }

    /// Reject an identifier byte glued directly to the end of a numeric literal —
    /// Postgres's `{integer|numeric|real}_junk` rules (`123abc`, `1e`, `1000_`,
    /// `0x1FG`). `$` is *not* an identifier-start byte, so `1$1` stays `1` then the
    /// `$1` parameter, matching Postgres.
    #[inline]
    fn reject_trailing_junk(&self, start: usize) -> Result<(), LexError> {
        if matches!(self.peek(), Some(b) if is_ident_start(b)) {
            return Err(lex_error(start, "trailing junk after numeric literal"));
        }
        Ok(())
    }

    /// Consume a maximal run of ASCII digits, allowing a `_` digit-group separator
    /// only *between* two digits. Returns whether any `_` was consumed. A leading,
    /// trailing, doubled, or decimal-adjacent `_` is **not** consumed — the run
    /// stops before it, leaving it to the next token; Postgres rejects those
    /// spellings outright, and pglet's tail then fails loud downstream (a stray
    /// identifier/number after the literal → `42601`), so the reject is preserved.
    fn scan_digit_run(&mut self) -> bool {
        let mut saw_underscore = false;
        let mut last_was_digit = false;
        loop {
            match self.peek() {
                Some(d) if d.is_ascii_digit() => {
                    self.pos += 1;
                    last_was_digit = true;
                }
                Some(b'_')
                    if last_was_digit
                        && matches!(self.peek_at(1), Some(d) if d.is_ascii_digit()) =>
                {
                    self.pos += 1;
                    saw_underscore = true;
                    last_was_digit = false;
                }
                _ => break,
            }
        }
        saw_underscore
    }

    /// Scan an operator or punctuation token (`b` is the byte at the cursor).
    ///
    /// The punctuation `scan.l` reads by a rule of its own — `,()[].;` and the `:`
    /// forms — is taken first, and everything built from `op_chars` goes through
    /// [`Self::scan_op_chars`], which is that scanner's `{operator}` rule.
    fn scan_operator(&mut self, start: usize, b: u8) -> Result<Token, LexError> {
        let kind = match b {
            b'(' => self.single(TokenKind::LParen),
            b')' => self.single(TokenKind::RParen),
            b'[' => self.single(TokenKind::LBracket),
            b']' => self.single(TokenKind::RBracket),
            b',' => self.single(TokenKind::Comma),
            b';' => self.single(TokenKind::Semicolon),
            b'.' => self.single(TokenKind::Dot),
            // `::` is `scan.l`'s `typecast` rule and `:` its `self` character; neither
            // `:` nor `.` is an `op_chars` byte, so no operator holds one.
            b':' if self.peek_at(1) == Some(b':') => self.double(TokenKind::Cast),
            b':' => self.single(TokenKind::Colon),
            _ if is_reference_op_char(b) => return self.scan_op_chars(start),
            _ => {
                // `scan.l`'s `{other}` rule: a one-character token no `gram.y` rule
                // takes, which is a syntax error there as it is here.
                return Err(lex_error(
                    start,
                    format!("unexpected character {:?}", char::from(b)),
                ));
            }
        };
        Ok(Token {
            kind,
            start,
            end: self.pos,
            decoded: None,
        })
    }

    /// Scan `scan.l`'s `{operator}` — a maximal run of `op_chars`, cut where a comment
    /// opener starts inside it, then trimmed of trailing `+`/`-`.
    ///
    /// The token is classified by the text that survives: a single `self` character or
    /// one of the five two-character tokens keeps the kind the rules ahead of
    /// `{operator}` would have given it (`scan.l` orders them before it, and flex takes
    /// the earlier rule on a tie), a spelling pglet's grammar knows takes that kind, and
    /// every other is [`TokenKind::Op`] — the generic operator token 18.6's `Op` is,
    /// which the parser resolves by its text and refuses `42883` when it names an
    /// operator pglet does not implement.
    ///
    /// The trimming rule is 18.6's, for SQL compatibility: `+` and `-` cannot end a
    /// multi-character operator unless it holds a character SQL operators do not use
    /// (`~!@#^&|`?%`), so `1 =- 1` lexes as `=` then `-1` while `2 ^- 1` is the operator
    /// `^-`, which does not exist (measured on 18.6, both).
    ///
    /// **What survives is refused past `NAMEDATALEN`.** `scan.l` ends the rule with
    /// `if (nchars >= NAMEDATALEN) yyerror("operator too long")`, over the length *after*
    /// the cut and the trim and after the one- and two-character early returns — so 63
    /// characters lex and 64 are `42601` there. That refusal is the **scanner's**, so it
    /// is raised here rather than deferred: 18.6 raises it in its raw parse, ahead of the
    /// statement's analysis and of any `54001` the statement owes. Measured on 18.6:
    /// `SELECT 1 <63 '*'> 1` is `42883` and `<64 '*'>` is `42601`, and `<63 '*'>++`
    /// trims back to 63 and is `42883`.
    fn scan_op_chars(&mut self, start: usize) -> Result<Token, LexError> {
        while matches!(self.peek(), Some(b) if is_reference_op_char(b)) {
            self.pos += 1;
        }
        let text = &self.source[start..self.pos];
        // A `/*` or `--` inside the run starts a comment there, so the operator stops
        // at the first of them.
        let cut = match (text.find("/*"), text.find("--")) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b).unwrap_or(text.len()),
        };
        let mut len = cut;
        let bytes = text.as_bytes();
        if len > 1
            && matches!(bytes[len - 1], b'+' | b'-')
            && !bytes[..len - 1].iter().any(|b| {
                matches!(
                    b,
                    b'~' | b'!' | b'@' | b'#' | b'^' | b'&' | b'|' | b'`' | b'?' | b'%'
                )
            })
        {
            while len > 1 && matches!(bytes[len - 1], b'+' | b'-') {
                len -= 1;
            }
        }
        self.pos = start + len;
        // `scan.l` runs the length check after its `self` and two-character early
        // returns, which no spelling this wide can take — every kind ahead of the generic
        // operator is one or two characters — so the order costs nothing to reproduce and
        // the check stands alone here.
        if len >= NAMEDATALEN {
            return Err(lex_error(start, "operator too long"));
        }
        Ok(Token {
            kind: operator_kind(&text[..len]),
            start,
            end: self.pos,
            decoded: None,
        })
    }

    /// Consume one byte and return the kind (single-char token helper).
    #[inline]
    const fn single(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    /// Consume two bytes and return the kind (two-char operator helper).
    #[inline]
    const fn double(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 2;
        kind
    }
}

/// The token kind the operator spelled `text` takes.
///
/// One table, read by the lexer alone: the spellings 18.6's scanner returns as tokens of
/// their own (its `self` characters and the five two-character rules ahead of
/// `{operator}`), then the spellings pglet's grammar has a kind for, then
/// [`TokenKind::Op`] — 18.6's generic `Op`, whose meaning is the operator's *name* and
/// which the parser resolves by text.
///
/// `|` is deliberately not a `self` character here: 18.6 lexes a lone `|` as an operator
/// (`SELECT 1 | 2` answers `3` through `int4or`), and the `self` set that holds it is
/// 20devel's, whose `gram.y` gained an `a_expr '|' a_expr` rule making the same operator
/// call. pglet implements no `|`, so it is `42883` either way.
fn operator_kind(text: &str) -> TokenKind {
    match text {
        "+" => TokenKind::Plus,
        "-" => TokenKind::Minus,
        "*" => TokenKind::Star,
        "/" => TokenKind::Slash,
        "%" => TokenKind::Percent,
        "^" => TokenKind::Caret,
        "<" => TokenKind::Lt,
        ">" => TokenKind::Gt,
        "=" => TokenKind::Eq,
        "<=" => TokenKind::Le,
        ">=" => TokenKind::Ge,
        "<>" | "!=" => TokenKind::NotEq,
        // `scan.l`'s `equals_greater`: a token of its own, which `gram.y` takes only in a
        // named function argument — a spelling pglet does not parse, so it is the
        // `42601` 18.6 answers for `SELECT 1 => 2` rather than an operator.
        "=>" => TokenKind::FatArrow,
        "||" => TokenKind::Concat,
        "@>" => TokenKind::Containment,
        "<@" => TokenKind::ContainedBy,
        "->" => TokenKind::Arrow,
        "->>" => TokenKind::ArrowText,
        "#>" => TokenKind::HashArrow,
        "#>>" => TokenKind::HashArrowText,
        "?" => TokenKind::Question,
        "?|" => TokenKind::QuestionPipe,
        "?&" => TokenKind::QuestionAmp,
        _ => TokenKind::Op,
    }
}

/// Whether `b` is one of `scan.l`'s `op_chars` — a byte 18.6 lexes into an operator.
#[inline]
const fn is_reference_op_char(b: u8) -> bool {
    matches!(
        b,
        b'~' | b'!'
            | b'@'
            | b'#'
            | b'^'
            | b'&'
            | b'|'
            | b'`'
            | b'?'
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'<'
            | b'>'
            | b'='
    )
}

/// Build a `42601` (syntax) `LexError` at `position` with `message`.
#[inline]
fn lex_error(position: usize, message: impl Into<String>) -> LexError {
    LexError::syntax(position, message)
}

/// Postgres's `NAMEDATALEN` — the width of a `name`, and the bound `scan.l`'s
/// `{operator}` rule refuses an operator spelling at (`nchars >= NAMEDATALEN` is
/// `operator too long`). It is *not* applied to identifiers here: 18.6 truncates those
/// to `NAMEDATALEN - 1` with a notice, which pglet does not do at all (`docs/parser.md`
/// § Known lexical / parse divergences).
const NAMEDATALEN: usize = 64;

/// The bit width at which a value is certainly past Postgres's `numeric` format — the
/// smallest `n` with `2^n > 10^131072`, 131072 integer digits being the most that
/// format's `int16` weight field can address (`make_result`'s `NUMERIC_WEIGHT_MAX`;
/// 18.6 answers a 131072-digit literal and refuses a 131073-digit one with
/// `22003 value overflows numeric format`, measured).
///
/// `2^435411` has 131072 decimal digits and `2^435412` has 131073, so a value at or
/// above `2^435412` cannot be a `numeric` whatever its digits are — which is what lets
/// [`Radix::decode`] decline a literal from the *count* of its significant digits. The
/// crossing is the constant's whole content and two too low refuses what 18.6 answers:
/// at `435_410` a 145 138-digit octal and a 435 411-digit binary literal of all `1`s —
/// 131 072 decimal digits each, and both answered on 18.6 — were `22003` here.
const NUMERIC_CAP_BITS: usize = 435_412;

/// The base one [`Decimal`] limb carries: nine decimal digits, the widest power of ten
/// a `u32` limb holds (`10^10` is past `u32::MAX`); with every [`Radix::chunk`]
/// multiplier at most `2^29`, a limb times a multiplier plus a carry stays inside a
/// `u64`.
const LIMB_BASE: u64 = 1_000_000_000;
/// Decimal digits per [`Decimal`] limb — `LIMB_BASE`'s exponent, and the zero-padding
/// width every limb but the most significant renders at.
const LIMB_DIGITS: usize = 9;

/// A non-negative integer as base-`10^9` limbs, least significant first — what a
/// radix-prefixed literal is accumulated into, since one may hold more digits than any
/// integer type carries (`Lexer::scan_radix_number`).
///
/// Limbs rather than single decimal digits because the accumulation is schoolbook, so its
/// cost is (digits consumed) × (limbs held): nine decimal digits a limb and one
/// [`Radix::chunk`] of source digits a step (7 hex, 9 octal, 29 binary) cut the work of
/// decoding the largest literal `numeric` can hold from tens of seconds to a fraction of
/// one, which is the shape 18.6's own `set_var_from_non_decimal_integer_str` has
/// (int64-wide digit groups over base-10000 limbs).
#[derive(Default)]
struct Decimal(Vec<u32>);

impl Decimal {
    /// Multiply by `mul` and add `add`, schoolbook, in place.
    ///
    /// `mul` must be at most `LIMB_BASE` and `add` below `mul`, which keeps every
    /// intermediate inside a `u64`: a limb is below `LIMB_BASE`, so the product is at
    /// most `10^18` and the carry below `LIMB_BASE`.
    fn push_chunk(&mut self, mul: u64, add: u64) {
        let mut carry = add;
        for limb in &mut self.0 {
            let value = u64::from(*limb) * mul + carry;
            *limb = u32::try_from(value % LIMB_BASE).unwrap_or(0);
            carry = value / LIMB_BASE;
        }
        while carry > 0 {
            self.0.push(u32::try_from(carry % LIMB_BASE).unwrap_or(0));
            carry /= LIMB_BASE;
        }
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some((top, rest)) = self.0.split_last() else {
            return f.write_str("0");
        };
        write!(f, "{top}")?;
        for limb in rest.iter().rev() {
            write!(f, "{limb:0LIMB_DIGITS$}")?;
        }
        Ok(())
    }
}

/// The base of a radix-prefixed integer literal (`0x`/`0o`/`0b`).
#[derive(Clone, Copy)]
enum Radix {
    Hex,
    Oct,
    Bin,
}

impl Radix {
    /// The numeric base (16 / 8 / 2).
    const fn base(self) -> u32 {
        match self {
            Self::Hex => 16,
            Self::Oct => 8,
            Self::Bin => 2,
        }
    }

    /// The digit value of `b` in this base, or `None` if `b` isn't a valid digit.
    #[inline]
    fn digit(self, b: u8) -> Option<u32> {
        char::from(b).to_digit(self.base())
    }

    /// The "no digits after the prefix" error message (`0x` / `0o` / `0b` alone) —
    /// Postgres's `hexfail`/`octfail`/`binfail` rules.
    const fn empty_message(self) -> &'static str {
        match self {
            Self::Hex => "invalid hexadecimal integer",
            Self::Oct => "invalid octal integer",
            Self::Bin => "invalid binary integer",
        }
    }

    /// How many bits one digit of this base carries (the base being a power of two).
    const fn bits(self) -> usize {
        match self {
            Self::Hex => 4,
            Self::Oct => 3,
            Self::Bin => 1,
        }
    }

    /// How many digits of this base go into one [`Decimal::push_chunk`] step, and the
    /// multiplier that many digits are worth — the largest power of the base at or
    /// below [`LIMB_BASE`], so the product with a limb still fits a `u64`.
    const fn chunk(self) -> (usize, u64) {
        match self {
            Self::Hex => (7, 268_435_456),  // 16^7
            Self::Oct => (9, 134_217_728),  // 8^9
            Self::Bin => (29, 536_870_912), // 2^29
        }
    }

    /// The decimal spelling of `body` — the literal's digit run, `_` separators
    /// included — or `None` when the count of its **significant** digits alone proves
    /// the value past what Postgres's `numeric` format can hold.
    ///
    /// The cap is the point of the `None`. Base conversion is quadratic in the digit
    /// count here and in 18.6 alike, and 18.6 stops its own the moment the accumulated
    /// value passes `NUMERIC_WEIGHT_MAX` — no further digit can make it representable.
    /// This declines before the first digit instead of during the run, and from a count
    /// alone — which is a *weaker* bound, not an earlier one: the *smallest* value `s`
    /// significant digits can spell is `base^(s - 1)` = `2^(bits × (s - 1))`, so once
    /// that reaches [`NUMERIC_CAP_BITS`] every spelling with that many significant
    /// digits is over the cap and the decode would be work spent on a value nothing can
    /// hold — while a *larger* value one digit below it still decodes, 18.6 refusing it
    /// on its weight. Below the cap the decode runs to the end and the over-cap verdict is
    /// `PgNumeric::parse`'s, which is where the 131072-digit bound is actually enforced
    /// — this only avoids the unbounded cost above it.
    ///
    /// **Significant, not counted:** 18.6 decides on the *accumulated weight*
    /// (`set_var_from_non_decimal_integer_str`), so a leading zero costs it nothing and
    /// `0x` + 108 853 zeros + `1` is `1` there. Counting the whole digit run would refuse
    /// it `22003` here. The run's leading `0`s — and any `_` among them — are therefore
    /// skipped before the count is judged, which also keeps them out of the
    /// accumulation: the work is the significant digits', as it is in 18.6. What the
    /// skip must not move is the accepted width of a literal whose digits are *all*
    /// significant, which is unchanged because its leading-zero run is empty.
    fn decode(self, body: &str, digits: usize) -> Option<String> {
        let bytes = body.as_bytes();
        let mut leading = 0usize;
        let mut start = 0usize;
        while let Some(&b) = bytes.get(start) {
            match b {
                b'0' => {
                    leading += 1;
                    start += 1;
                }
                b'_' => start += 1, // a separator, which the scan already validated
                _ => break,
            }
        }
        let significant = digits - leading;
        if self.bits().saturating_mul(significant.saturating_sub(1)) >= NUMERIC_CAP_BITS {
            return None;
        }
        let base = u64::from(self.base());
        let (per_chunk, full) = self.chunk();
        let mut value = Decimal::default();
        let mut pending = 0u64;
        let mut held = 0usize;
        for &b in &bytes[start..] {
            let Some(d) = self.digit(b) else {
                continue; // a `_` separator, which the scan already validated
            };
            pending = pending * base + u64::from(d);
            held += 1;
            if held == per_chunk {
                value.push_chunk(full, pending);
                pending = 0;
                held = 0;
            }
        }
        if held > 0 {
            value.push_chunk(base.pow(u32::try_from(held).unwrap_or(0)), pending);
        }
        Some(value.to_string())
    }
}

/// Whether `b` may start a bare identifier: an ASCII letter, `_`, or any
/// non-ASCII byte (the lead/continuation of a Unicode identifier char).
#[inline]
const fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

/// Whether `b` may continue a bare identifier (adds digits and `$`).
#[inline]
const fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}
