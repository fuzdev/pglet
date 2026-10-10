//! Token kinds, the `Keyword` set, and the perfect-hash keyword table.
//!
//! Tokens are `{kind, start, end}` byte spans into the source plus an optional
//! `decoded` payload — owned only when a literal needs a transform (`''`/`""`
//! collapse, `E'…'`/`U&'…'` escapes, dollar-quoted bodies). Everything else is
//! zero-copy: the value is recovered from the source slice on demand (`Token::text`).

use std::fmt;

/// A lexed token: its kind plus the byte span `[start, end)` into the source.
///
/// `decoded` holds the resolved literal value only when the raw lexeme isn't the
/// value verbatim (escapes, doubled quotes, dollar-quote delimiters). For a plain
/// `'…'` string or an undoubled `"…"` identifier it stays `None` and the value is
/// the delimiter-stripped source slice (see `Token::text`).
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
    pub decoded: Option<String>,
}

impl Token {
    /// The semantic text of a value-bearing token (`String`, `QuotedIdentifier`,
    /// `Identifier`, `Number`): the decoded buffer when present, else the source
    /// slice with single-char string/identifier delimiters stripped.
    ///
    /// Bare identifiers are returned verbatim (not case-folded) — folding to
    /// lower case is [`Token::name`]'s job, since `"…"` identifiers are
    /// case-sensitive.
    #[inline]
    pub fn text<'s>(&'s self, source: &'s str) -> &'s str {
        if let Some(decoded) = &self.decoded {
            return decoded;
        }
        match self.kind {
            // `None` here only ever means a plain `'…'` / undoubled `"…"` — the
            // delimiters are one byte each side (E'/U&'/dollar-quote always decode).
            TokenKind::String | TokenKind::QuotedIdentifier => {
                &source[self.start + 1..self.end - 1]
            }
            _ => &source[self.start..self.end],
        }
    }

    /// The token's text **as a name**: ASCII-downcased when the source wrote it
    /// unquoted, verbatim when it wrote it `"…"`-quoted.
    ///
    /// This is Postgres's rule. Postgres downcases an unquoted identifier in the
    /// scanner (`downcase_truncate_identifier`) and takes a `"…"`-quoted one exactly,
    /// and the comparisons after that point are plain byte comparisons. Folding at the
    /// read rather than at each comparison is what makes the two directions agree: a
    /// downstream `eq_ignore_ascii_case` cannot distinguish `"PUBLIC"` from `public`,
    /// and a downstream `==` cannot resolve `FROM FOO` against a table created as
    /// `Foo`.
    ///
    /// This is the rule's implementation for an **identifier-position** token.
    /// `parser::split_name_literal` implements the same rule for a *name literal*
    /// (`nextval('…')`), and the two must move together or the same relation gets two
    /// names. The contextual-word matchers (`Parser::at_ident`,
    /// `parse_column_default`'s `try_function_default`) fold in place instead, and are
    /// reached only for an unquoted `Identifier` token, so a quoted spelling cannot
    /// match them at all. `parse_copy_header_arg` also folds in place, and unlike
    /// those it folds a quoted token too — deliberately, since a `HEADER` argument is
    /// a value and not a name.
    ///
    /// A **keyword** token folds too. Postgres's scanner classifies a word as a
    /// keyword only after downcasing it, so a non-reserved keyword used as a name
    /// (`CREATE TABLE Sequence`) reaches the catalog lowercased exactly as an
    /// ordinary identifier does. A `String` or `Number` token is returned verbatim:
    /// those are value positions, where case-insensitivity (if any) belongs to the
    /// value grammar, not to identifier folding.
    ///
    /// Only ASCII `A`–`Z` fold. Measured on the 18.6 oracle (UTF-8): `CREATE TABLE
    /// ÉFOO` is not reachable as `éfoo` there either, and the source agrees —
    /// `scansup.c`'s `downcase_identifier` folds a high-bit byte (through `tolower`)
    /// only when the database encoding is single-byte. So the width is a property of
    /// the reference database's encoding: a single-byte encoding could differ. pglet
    /// models no per-database encoding.
    ///
    /// That width binds anything that folds a name **again** downstream: a re-fold is
    /// only safe at the width of the fold that produced the name it re-folds, or it
    /// takes the letters this one handed through. The catalog type key is folded again
    /// that way (`executor::ddl::execute_create_type` and its two readers) — see its
    /// doc comment for the pair of unquoted `CREATE TYPE` statements a wider one costs.
    #[inline]
    #[must_use]
    pub fn name(&self, source: &str) -> String {
        let text = self.text(source);
        match self.kind {
            TokenKind::Identifier | TokenKind::Keyword(_) => text.to_ascii_lowercase(),
            _ => text.to_string(),
        }
    }
}

/// The lexical category of a token.
///
/// The punctuation and the operator spellings pglet's grammar reads have a kind each;
/// every other operator is [`Op`](Self::Op), 18.6's generic operator token, whose name
/// is its source text. The lexer classifies an operator by `scan.l`'s rules (`core`'s
/// `scan_op_chars`), so the token boundaries are the reference scanner's and a name
/// pglet implements no operator for reaches the parser, which refuses it `42883` where
/// 18.6's analysis does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// A bare (unquoted) identifier — downcased by [`Token::name`].
    Identifier,
    /// A `"…"`-quoted identifier — case-sensitive, verbatim.
    QuotedIdentifier,
    /// A reserved/known keyword.
    Keyword(Keyword),
    /// A numeric literal (`123`, `1.5`, `.5`, `1e10`); value via the source slice.
    Number,
    /// A string literal in any form (`'…'`, `E'…'`, `U&'…'`, `$tag$…$tag$`).
    String,
    /// A positional parameter placeholder `$N` (1-based).
    Param(u32),

    // operators
    Cast,          // ::
    Containment,   // @>
    ContainedBy,   // <@
    Arrow,         // ->
    ArrowText,     // ->>
    HashArrow,     // #> (JSON path extract → jsonb)
    HashArrowText, // #>> (JSON path extract → text)
    Question,      // ? (JSON key/element exists)
    QuestionPipe,  // ?| (JSON any-key exists)
    QuestionAmp,   // ?& (JSON all-keys exist)
    Concat,        // ||
    /// A generic operator — 18.6's `Op`, the token every `op_chars` spelling outside the
    /// kinds above lexes to (`~`, `&`, `#`, `~~`, `^@`, `!`, a backtick, `#-`, …).
    ///
    /// It carries no payload: the operator's **name** is its source text, which the
    /// parser reads (`TokenStream::peek_text`) to resolve it. A name pglet implements no
    /// operator for is `42883` at the statement's analysis, as it is on 18.6.
    Op,
    /// `=>` — `scan.l`'s `equals_greater`, which `gram.y` takes only in a named function
    /// argument. pglet parses no such argument, so the token exists to be the syntax
    /// error 18.6 answers rather than to be read as `=` then `>`.
    FatArrow,
    Eq,      // =
    NotEq,   // <> or !=
    Lt,      // <
    Le,      // <=
    Gt,      // >
    Ge,      // >=
    Plus,    // +
    Minus,   // -
    Star,    // *
    Slash,   // /
    Percent, // %
    Caret,   // ^

    // punctuation
    LParen,    // (
    RParen,    // )
    LBracket,  // [
    RBracket,  // ]
    Comma,     // ,
    Semicolon, // ;
    Dot,       // .
    Colon,     // : (lone — array-slice colon; the parser decides validity)

    /// End of input.
    Eof,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identifier => f.write_str("identifier"),
            Self::QuotedIdentifier => f.write_str("quoted identifier"),
            Self::Keyword(kw) => write!(f, "'{}'", kw.as_str()),
            Self::Number => f.write_str("number"),
            Self::String => f.write_str("string"),
            Self::Param(n) => write!(f, "'${n}'"),
            Self::Cast => f.write_str("'::'"),
            Self::Containment => f.write_str("'@>'"),
            Self::ContainedBy => f.write_str("'<@'"),
            Self::Arrow => f.write_str("'->'"),
            Self::ArrowText => f.write_str("'->>'"),
            Self::HashArrow => f.write_str("'#>'"),
            Self::HashArrowText => f.write_str("'#>>'"),
            Self::Question => f.write_str("'?'"),
            Self::QuestionPipe => f.write_str("'?|'"),
            Self::QuestionAmp => f.write_str("'?&'"),
            Self::Concat => f.write_str("'||'"),
            Self::Op => f.write_str("operator"),
            Self::FatArrow => f.write_str("'=>'"),
            Self::Eq => f.write_str("'='"),
            Self::NotEq => f.write_str("'<>'"),
            Self::Lt => f.write_str("'<'"),
            Self::Le => f.write_str("'<='"),
            Self::Gt => f.write_str("'>'"),
            Self::Ge => f.write_str("'>='"),
            Self::Plus => f.write_str("'+'"),
            Self::Minus => f.write_str("'-'"),
            Self::Star => f.write_str("'*'"),
            Self::Slash => f.write_str("'/'"),
            Self::Percent => f.write_str("'%'"),
            Self::Caret => f.write_str("'^'"),
            Self::LParen => f.write_str("'('"),
            Self::RParen => f.write_str("')'"),
            Self::LBracket => f.write_str("'['"),
            Self::RBracket => f.write_str("']'"),
            Self::Comma => f.write_str("','"),
            Self::Semicolon => f.write_str("';'"),
            Self::Dot => f.write_str("'.'"),
            Self::Colon => f.write_str("':'"),
            Self::Eof => f.write_str("end of input"),
        }
    }
}

/// A keyword's Postgres class, from `kwlist.h` — the property that decides where
/// a keyword may appear as an identifier.
///
/// pglet derives keyword *reservation* from this single source of truth: a word
/// is a valid bare column / table name exactly when Postgres's `ColId` grammar
/// admits it, i.e. when it is [`Unreserved`](Self::Unreserved) or
/// [`ColName`](Self::ColName). The two non-`ColId` classes
/// ([`Reserved`](Self::Reserved), [`TypeFuncName`](Self::TypeFuncName)) are the
/// reserved set as far as a bare-name position is concerned — so `is`/`like`/
/// `left` (`TypeFuncName`) are rejected as bare columns by Postgres *and* pglet.
/// See `parser::is_reserved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgClass {
    /// Fully reserved — never an identifier without quoting (`select`, `from`).
    Reserved,
    /// Usable as any identifier — column, table, or alias name (`update`, `by`).
    Unreserved,
    /// Usable as a column / table name, but not a function / type name
    /// (`values`, `between`).
    ColName,
    /// Usable as a function / type name, but **not** as a bare column / table
    /// name (`is`, `like`, `left`, `join`) — `ColId` excludes it.
    TypeFuncName,
}

/// Generates the `Keyword` enum, its `as_str`, its [`PgClass`] mapping, the
/// `ALL_KEYWORDS` roster, and the perfect-hash lookup table from a single list —
/// one source of truth, no drift between them.
macro_rules! keywords {
	($($variant:ident => $text:literal : $class:ident),+ $(,)?) => {
		/// A recognized SQL keyword.
		///
		/// Each entry carries its Postgres [`PgClass`] (from `kwlist.h`); the
		/// parser derives reservation from `Keyword::pg_class`, so the
		/// classification lives beside the spelling rather than in a separate
		/// hand-maintained set.
		#[derive(Debug, Clone, Copy, PartialEq, Eq)]
		pub enum Keyword {
			$($variant),+
		}

		impl Keyword {
			/// The canonical lower-case spelling.
			#[inline]
			pub const fn as_str(self) -> &'static str {
				match self {
					$(Self::$variant => $text),+
				}
			}

			/// This keyword's Postgres class (`kwlist.h`) — the source of truth
			/// for whether it may serve as a bare identifier.
			///
			/// An `O(1)` array lookup keyed by the enum discriminant. `self as
			/// usize` is always in range for this fieldless enum
			/// (`0..ALL_KEYWORDS.len()`), so the slice bounds check always
			/// passes and is typically elided — keeping this off the parse
			/// hot path's critical chain.
			#[inline]
			pub const fn pg_class(self) -> PgClass {
				KEYWORD_CLASSES[self as usize]
			}
		}

		/// The `PgClass` of each keyword, indexed by enum discriminant — the data
		/// table behind [`Keyword::pg_class`].
		const KEYWORD_CLASSES: &[PgClass] = &[$(PgClass::$class),+];

		/// Every recognized keyword, in declaration order. Lets a conformance
		/// sweep iterate the whole set (the keyword-as-identifier tests) without a
		/// second hand-maintained list that could drift from the enum.
		pub const ALL_KEYWORDS: &[Keyword] = &[$(Keyword::$variant),+];

		static KEYWORDS: phf::Map<&'static str, Keyword> = phf::phf_map! {
			$($text => Keyword::$variant),+
		};
	};
}

// Each entry's class is its Postgres `kwlist.h` category (verified against
// `src/include/parser/kwlist.h` in the PostgreSQL source). The parser reserves everything that
// is **not** a valid `ColId` — i.e. `Reserved` and `TypeFuncName` — matching
// Postgres's own bare-name acceptance exactly.
keywords! {
    // statements / DML
    Select => "select" : Reserved, Insert => "insert" : Unreserved,
    Update => "update" : Unreserved, Delete => "delete" : Unreserved,
    Into => "into" : Reserved, Values => "values" : ColName,
    Set => "set" : Unreserved, Returning => "returning" : Reserved,
    // DDL
    Create => "create" : Reserved, Drop => "drop" : Unreserved,
    Alter => "alter" : Unreserved, Table => "table" : Reserved,
    Truncate => "truncate" : Unreserved,
    Copy => "copy" : Unreserved, To => "to" : Reserved,
    Index => "index" : Unreserved, View => "view" : Unreserved,
    Sequence => "sequence" : Unreserved, Add => "add" : Unreserved,
    Column => "column" : Reserved, Constraint => "constraint" : Reserved,
    Primary => "primary" : Reserved, Key => "key" : Unreserved,
    Unique => "unique" : Reserved, Check => "check" : Reserved,
    Foreign => "foreign" : Reserved, References => "references" : Reserved,
    Default => "default" : Reserved, Cascade => "cascade" : Unreserved,
    Restrict => "restrict" : Unreserved, Action => "action" : Unreserved,
    If => "if" : Unreserved,
    // clauses
    From => "from" : Reserved, Where => "where" : Reserved,
    Group => "group" : Reserved, By => "by" : Unreserved, Having => "having" : Reserved,
    Order => "order" : Reserved, Limit => "limit" : Reserved,
    Offset => "offset" : Reserved, Distinct => "distinct" : Reserved,
    As => "as" : Reserved, Asc => "asc" : Reserved, Desc => "desc" : Reserved,
    Nulls => "nulls" : Unreserved, First => "first" : Unreserved,
    Last => "last" : Unreserved, On => "on" : Reserved, Using => "using" : Reserved,
    With => "with" : Reserved, Recursive => "recursive" : Unreserved,
    // conflict
    Conflict => "conflict" : Unreserved, Do => "do" : Reserved,
    Nothing => "nothing" : Unreserved,
    // joins
    Join => "join" : TypeFuncName, Inner => "inner" : TypeFuncName,
    Left => "left" : TypeFuncName, Right => "right" : TypeFuncName,
    Full => "full" : TypeFuncName, Outer => "outer" : TypeFuncName,
    Cross => "cross" : TypeFuncName, Natural => "natural" : TypeFuncName,
    // predicates / logic
    And => "and" : Reserved, Or => "or" : Reserved, Not => "not" : Reserved,
    Null => "null" : Reserved, True => "true" : Reserved, False => "false" : Reserved,
    Is => "is" : TypeFuncName, In => "in" : Reserved, Like => "like" : TypeFuncName,
    // The postfix null tests `x ISNULL` / `x NOTNULL` (`gram.y`'s historical spellings
    // of `IS [NOT] NULL`). Keywords so the operator loop can read them; their class
    // keeps them out of every bare-name position, as `kwlist.h`'s does.
    Isnull => "isnull" : TypeFuncName, Notnull => "notnull" : TypeFuncName,
    Ilike => "ilike" : TypeFuncName, Between => "between" : ColName,
    Any => "any" : Reserved, All => "all" : Reserved, Some => "some" : Reserved,
    Exists => "exists" : ColName,
    // case
    Case => "case" : Reserved, When => "when" : Reserved, Then => "then" : Reserved,
    Else => "else" : Reserved, End => "end" : Reserved,
    // set operations
    Union => "union" : Reserved, Intersect => "intersect" : Reserved,
    Except => "except" : Reserved,
    // cast / array / time
    Cast => "cast" : Reserved, Collate => "collate" : Reserved,
    Array => "array" : Reserved, At => "at" : Unreserved,
    // transactions
    Begin => "begin" : Unreserved, Commit => "commit" : Unreserved,
    Rollback => "rollback" : Unreserved, Start => "start" : Unreserved,
    Transaction => "transaction" : Unreserved, Savepoint => "savepoint" : Unreserved,
    Release => "release" : Unreserved,
    // locking
    For => "for" : Reserved,
    // privileges / comments (pg_dump preamble tolerance; statement-leading words)
    Grant => "grant" : Reserved, Revoke => "revoke" : Unreserved,
    Comment => "comment" : Unreserved,
    // misc
    Show => "show" : Unreserved, Lateral => "lateral" : Reserved, No => "no" : Unreserved,
    // string-function special forms — COL_NAME_KEYWORD in Postgres (still valid bare
    // column names). Their `SUBSTRING(x FROM n FOR m)` / `TRIM([BOTH|…] c FROM x)` /
    // `POSITION(sub IN str)` grammars are recognized in `parse_primary` (paren-guarded);
    // BOTH/LEADING/TRAILING stay contextual identifiers, not reserved words.
    Substring => "substring" : ColName, Trim => "trim" : ColName,
    Position => "position" : ColName,
    // `EXTRACT(field FROM source)` — also a COL_NAME_KEYWORD (bare `extract` stays a
    // usable column name); its grammar is recognized in `parse_primary` (paren-guarded).
    Extract => "extract" : ColName,
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The byte length of the longest keyword spelling — `transaction` (11). Used to
/// size the case-folding stack buffer; an identifier longer than this can't be a
/// keyword, so the lookup short-circuits without touching the map.
const MAX_KEYWORD_LEN: usize = 12;

/// Case-insensitive keyword lookup, allocation-free.
///
/// SQL keywords are case-insensitive ASCII. The identifier is ASCII-lower-cased
/// into a fixed stack buffer (a non-ASCII byte means it can't be a keyword) and
/// matched against the perfect-hash table.
#[inline]
pub fn keyword_kind(ident: &str) -> Option<Keyword> {
    let bytes = ident.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_KEYWORD_LEN {
        return None;
    }
    let mut buf = [0u8; MAX_KEYWORD_LEN];
    for (dst, &b) in buf.iter_mut().zip(bytes) {
        if !b.is_ascii() {
            return None;
        }
        *dst = b.to_ascii_lowercase();
    }
    // `buf[..len]` is pure ASCII (guarded above), so this never fails.
    let lowered = std::str::from_utf8(&buf[..bytes.len()]).ok()?;
    KEYWORDS.get(lowered).copied()
}
