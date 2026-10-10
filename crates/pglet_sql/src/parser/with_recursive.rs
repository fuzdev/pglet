//! A `WITH RECURSIVE` list, resolved in **dependency order**.
//!
//! Under `RECURSIVE` every name in the list is visible to every body, so a CTE may read
//! one declared after it, and 18.6's `transformWithClause` analyses the list in the order
//! `makeDependencyGraph` / `TopologicalSort` derive from the raw parse tree, after
//! `checkWellFormedRecursion` has judged each self-referencing item. pglet parses and
//! plans a body in one act, so a body cannot be planned before the bodies it reads; the
//! raw facts those three phases read are therefore taken by a **token pre-scan** of the
//! list ([`Parser::scan_with_body`]), and the bodies are then parsed for real in the
//! sorted order by restoring the stream to each one.
//!
//! The pre-scan aims at what `makeDependencyGraphWalker` reads — an unqualified name in
//! a **range-variable position** (a `FROM` / `JOIN` item, a comma-separated `FROM`
//! item, a `DELETE`'s `USING` item, an `INSERT INTO` target — with a column list after
//! it or not — an `UPDATE` target, a `FOR … OF` name) — and not at a column, a
//! qualified name, a function call (a `FROM` item's name followed by `(`) or a `VALUES`
//! row — nor at the word after an `AS`, a label or a name even when it is spelled like
//! a clause keyword. A reference it misses, in a `FROM` position, to a name not resolved
//! yet is refused by the parse rather than guessed (`Parser::pending_ctes`); one it
//! invents costs an ordering constraint, a cycle 18.6 does not see, or — naming the CTE's own
//! name — a well-formedness verdict 18.6 does not reach.
//!
//! What it cannot reproduce is 18.6's **raw parse**, which reports a syntax error ahead
//! of every phase here. When resolving the list fails, the list's bodies are parsed again
//! in declaration order and then the statement body, and the first **syntax** error
//! found outranks the failure ([`Parser::first_syntax_error`]); a non-syntax error ends
//! only its own body's search, so a syntax error behind one in the same body, or behind
//! one in the statement body, is not found. A list the pre-scan cannot read takes the
//! declaration-order walk instead, which reports the syntax error as it always has.

use super::Parser;
use super::select::QueryClauses;
use super::stream::StreamCheckpoint;
use crate::lexer::{Keyword, TokenKind};
use crate::{Error, Result};

/// One `WITH RECURSIVE` item as the pre-scan read it.
struct ScannedItem<'a> {
    name: String,
    col_aliases: Vec<String>,
    /// The cursor at the body's first token, just past its `(` and past each further
    /// `(` that [`Parser::skip_body_wrappers`] finds enclosing a whole `SELECT` body
    /// (`AS ((SELECT …))`, which 18.6's grammar reads as the same query).
    body: StreamCheckpoint<'a>,
    /// How many of those enclosing `(` the body carries, each closed by a `)` after it.
    wraps: usize,
    shape: BodyShape,
}

/// The raw-tree facts about one body that 18.6's dependency and well-formedness phases
/// read.
#[derive(Default)]
struct BodyShape {
    /// Each unqualified name in a range-variable position, in order.
    relations: Vec<RelationRef>,
    /// The body is an `INSERT` / `UPDATE` / `DELETE`.
    modifying: bool,
    /// The body's first token is `(`, so with no top-level set operator its top-level
    /// shape is inside the parentheses, where this scan does not look.
    leads_with_paren: bool,
    /// The top-level set operators, in order, with their token ordinals.
    set_ops: Vec<(Keyword, usize)>,
    /// The first, in `checkWellFormedRecursion`'s order, of the top-level `ORDER BY`,
    /// `OFFSET`, `LIMIT` / `FETCH` and locking clauses the body has.
    decoration: Option<Decoration>,
    /// The token ordinal of the body's first top-level `ORDER` / `OFFSET` / `LIMIT` /
    /// `FETCH` / `FOR` — where the clauses over the whole query begin, which is where a
    /// decoration's grammar is checked from.
    tail_at: Option<usize>,
}

/// A clause over a whole query that 18.6 refuses over a recursive one, in the order
/// `checkWellFormedRecursion` tests them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Decoration {
    OrderBy,
    Offset,
    Limit,
    Locking,
}

impl Decoration {
    /// The clause as 18.6's refusal names it.
    const fn spelling(self) -> &'static str {
        match self {
            Self::OrderBy => "ORDER BY",
            Self::Offset => "OFFSET",
            Self::Limit => "LIMIT",
            Self::Locking => "FOR UPDATE/SHARE",
        }
    }
}

impl BodyShape {
    /// The set operator at the top of the body's tree — the last `UNION`/`EXCEPT`
    /// (equal precedence, left-associative), else the last `INTERSECT` — with its
    /// ordinal, which splits the body into its left and right arms.
    fn top_set_op(&self) -> Option<(Keyword, usize)> {
        let loose = |k: &Keyword| matches!(k, Keyword::Union | Keyword::Except);
        self.set_ops
            .iter()
            .rev()
            .find(|(k, _)| loose(k))
            .or_else(|| self.set_ops.last())
            .copied()
    }

    /// Record a top-level decoration, keeping the one 18.6 tests first.
    fn decorate(&mut self, top: bool, at: usize, decoration: Decoration) {
        if top {
            self.decoration = Some(self.decoration.map_or(decoration, |d| d.min(decoration)));
        }
        self.tail(top, at);
    }

    /// Record where a top-level clause over the whole query begins, keeping the first.
    fn tail(&mut self, top: bool, at: usize) {
        if top {
            self.tail_at.get_or_insert(at);
        }
    }

    /// Whether the body names `name` in a range-variable position.
    fn reads(&self, name: &str) -> bool {
        self.relations.iter().any(|r| r.name == name)
    }
}

/// One name in a range-variable position, with the context
/// `checkWellFormedRecursionWalker` judges a self-reference by.
struct RelationRef {
    name: String,
    /// Its token ordinal in the body, which places it in the left or the right arm.
    at: usize,
    /// An expression sub-link encloses it.
    sublink: bool,
    /// It sits on the nullable side of an outer join.
    nullable: bool,
}

/// The outer-join kind a `JOIN` keyword is waiting for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingJoin {
    Left,
    Right,
    Full,
}

/// Per-parenthesis state of the pre-scan.
// One flag per grammar position the scan tracks; each is read at its own token, and a
// state machine over their product would hide which token reads which.
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct Frame {
    /// A `SELECT` / `DELETE` / `UPDATE` began at this level, so a `FROM` here is a
    /// clause and not an expression's (`EXTRACT(… FROM …)`).
    query: bool,
    /// Inside a `FROM` / `USING` list, where a comma begins another item.
    from_list: bool,
    /// After `FOR <strength>`, where `OF` begins a list of relation names.
    locking: bool,
    /// Inside a `FOR … OF` list.
    locking_of: bool,
    /// Tokens seen at this level, so the first one can be told apart (`UPDATE t` from
    /// `FOR UPDATE`).
    tokens: usize,
    /// This level is a `DELETE` whose `USING` list has not begun: the `USING` that
    /// introduces relations here. A join's `USING` column list and a sort operator's
    /// `USING` (`ORDER BY a USING <`) name none; a `MERGE`'s, which does, pglet does not
    /// parse.
    delete_using: bool,
    /// An expression sub-link encloses this level: a parenthesis opened anywhere but at
    /// a `FROM` item's position, or one nested under such a parenthesis.
    sublink: bool,
    /// This level is a `FROM` item on the nullable side of an outer join, or is nested
    /// in one.
    nullable: bool,
    /// Where in [`BodyShape::relations`] the current comma-separated `FROM` item began —
    /// everything a `RIGHT` / `FULL JOIN` makes nullable on its left.
    item_start: usize,
    /// The `LEFT` / `RIGHT` / `FULL` read ahead of a `JOIN`.
    pending_join: Option<PendingJoin>,
    /// The next `FROM` operand is a `LEFT` / `FULL JOIN`'s nullable right side.
    next_nullable: bool,
}

impl Frame {
    const fn end_list(&mut self) {
        self.from_list = false;
        self.locking_of = false;
    }
}

impl<'a> Parser<'a> {
    /// Resolve a `WITH RECURSIVE` list (`WITH RECURSIVE` already consumed) in dependency
    /// order, and return `true`; or restore the stream and return `false` for the
    /// declaration-order walk, when the pre-scan cannot read the list.
    ///
    /// # Errors
    ///
    /// In 18.6's phase order: `0A000 mutual recursion between WITH items is not
    /// implemented` for a dependency cycle of more than one item; then, per
    /// self-referencing item in sorted order, the well-formedness refusals
    /// ([`Self::check_well_formed_recursion`]); then whatever a body's own parse raises.
    pub(super) fn parse_with_recursive_list(&mut self) -> Result<bool> {
        let start = self.ts.checkpoint();
        let Ok(items) = self.scan_with_list() else {
            self.ts.restore(start);
            return Ok(false);
        };
        let end = self.ts.checkpoint();
        let verdict = match self.resolve_with_items(&items) {
            // A deferred verdict fires once the list has parsed — and still behind a
            // syntax error in the statement body, which 18.6's raw parse reports first.
            Ok(()) => match self.take_with_clause_verdict() {
                None => {
                    self.ts.restore(end);
                    return Ok(true);
                }
                Some(verdict) => verdict,
            },
            // 18.6 raw-parses the whole statement before any of these phases, so its
            // first syntax error, in text order, outranks `err` — and a duplicate name,
            // then an arity error an item resolved before the failing one recorded,
            // outrank what the phases after them raise.
            Err(err @ Error::Syntax { .. }) => err,
            Err(err) => self.take_with_clause_verdict().unwrap_or(err),
        };
        Err(self.first_syntax_error(&items, end).unwrap_or(verdict))
    }

    /// Resolve the scanned items: `transformWithClause`'s phases in its order.
    fn resolve_with_items(&mut self, items: &[ScannedItem<'a>]) -> Result<()> {
        // A duplicate outranks every phase below, so with one the list is parsed in
        // declaration order with no sort and no well-formedness verdict; the recorded
        // `42712` fires once the list has parsed.
        let duplicate = items
            .iter()
            .enumerate()
            .find(|(i, a)| items[i + 1..].iter().any(|b| b.name == a.name));
        let order = if let Some((_, item)) = duplicate {
            self.with_clause_verdict
                .get_or_insert_with(|| Error::DuplicateCteName {
                    name: item.name.clone(),
                });
            (0..items.len()).collect()
        } else {
            let order = topological_sort(items)?;
            for &i in &order {
                self.check_well_formed_recursion(&items[i])?;
            }
            self.pending_ctes = items.iter().map(|item| item.name.clone()).collect();
            order
        };
        for i in order {
            self.parse_with_item(&items[i])?;
            self.pending_ctes.retain(|n| *n != items[i].name);
        }
        Ok(())
    }

    /// Parse one scanned item's body and register it.
    fn parse_with_item(&mut self, item: &ScannedItem<'a>) -> Result<()> {
        self.ts.restore(item.body.clone());
        let mark = self.touched_tables.len();
        if item.shape.reads(&item.name) && !item.shape.modifying {
            self.parse_recursive_cte_body(item.name.clone(), &item.col_aliases)?;
        } else {
            // Not self-referencing: an ordinary CTE, as 18.6's `cterecursive = false`
            // item is — a `UNION` chain, an `ORDER BY` and a data-modifying body all
            // allowed.
            let plan = self.parse_cte_body()?;
            self.register_cte(item.name.clone(), plan, item.col_aliases.clone())?;
        }
        self.record_cte_tables(item.name.clone(), mark);
        for _ in 0..=item.wraps {
            self.ts.expect(&TokenKind::RParen)?;
        }
        Ok(())
    }

    /// The first **syntax** error in the statement — each item's body in declaration
    /// order, then the statement body after the list — standing in for the raw parse
    /// 18.6 runs before analysing anything. A non-syntax error ends only its own body's
    /// search; one inside the statement body ends the search.
    fn first_syntax_error(
        &mut self,
        items: &[ScannedItem<'a>],
        end: StreamCheckpoint<'a>,
    ) -> Option<Error> {
        self.pending_ctes.clear();
        self.ctes.clear();
        self.next_cte_rank = 0;
        self.materialized_ctes.clear();
        self.with_clause_arity_verdict = None;
        for item in items {
            if let Err(err @ Error::Syntax { .. }) = self.parse_with_item(item) {
                return Some(err);
            }
        }
        self.ts.restore(end);
        match self.parse_with_statement_body() {
            Err(err @ Error::Syntax { .. }) => Some(err),
            _ => None,
        }
    }

    /// Scan the whole list: each item's name, column-alias list and body, the cursor
    /// left past the last item.
    fn scan_with_list(&mut self) -> Result<Vec<ScannedItem<'a>>> {
        let mut items = Vec::new();
        loop {
            let name = self.expect_name("a CTE name")?;
            let col_aliases = if self.ts.at(&TokenKind::LParen) {
                self.parse_paren_name_list()?
            } else {
                Vec::new()
            };
            self.ts.expect_kw(Keyword::As)?;
            if self.ts.eat_kw(Keyword::Not)? {
                self.expect_ident("materialized")?;
            } else {
                self.eat_ident("materialized")?;
            }
            self.ts.expect(&TokenKind::LParen)?;
            let wraps = self.skip_body_wrappers()?;
            let body = self.ts.checkpoint();
            let shape = self.scan_with_body()?;
            for _ in 0..=wraps {
                self.ts.expect(&TokenKind::RParen)?;
            }
            items.push(ScannedItem {
                name,
                col_aliases,
                body,
                wraps,
                shape,
            });
            if !self.ts.eat(&TokenKind::Comma)? {
                return Ok(items);
            }
        }
    }

    /// Step past each `(` that encloses a whole `SELECT` body — its `)` is followed by
    /// the body's own — and return how many: `AS ((SELECT …))` is the query
    /// `AS (SELECT …)` in 18.6's grammar (`select_with_parens`), so the scan and the
    /// parse both read the inner query. A parenthesis followed by anything but a
    /// `SELECT` or a further parenthesis is left alone: a data-modifying statement has
    /// no parenthesized form, and a `(…)` followed by clauses of its own is not the
    /// whole body.
    fn skip_body_wrappers(&mut self) -> Result<usize> {
        let mut wraps = 0;
        while self.ts.at(&TokenKind::LParen) {
            let start = self.ts.checkpoint();
            self.ts.bump()?;
            let query = self.ts.at_kw(Keyword::Select) || self.ts.at(&TokenKind::LParen);
            let mut depth = 1usize;
            while depth > 0 {
                if self.ts.at_eof() {
                    return Err(self.ts.expected("')'"));
                }
                match self.ts.advance()?.kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => depth -= 1,
                    _ => {}
                }
            }
            let whole = query && self.ts.at(&TokenKind::RParen);
            self.ts.restore(start);
            if !whole {
                break;
            }
            self.ts.bump()?;
            wraps += 1;
        }
        Ok(wraps)
    }

    /// Read one body's tokens up to (not past) its closing `)`, collecting the
    /// [`BodyShape`] facts.
    fn scan_with_body(&mut self) -> Result<BodyShape> {
        let mut shape = BodyShape {
            leads_with_paren: self.ts.at(&TokenKind::LParen),
            ..BodyShape::default()
        };
        let mut frames = vec![Frame::default()];
        let mut expect_relation = false;
        // The relation position is an `INSERT INTO` target, whose name may be followed
        // by a `(` — its column list, or a parenthesized row source — where a `(` after
        // a `FROM` item's name makes it a function call (`FROM f(x)`; a table's alias
        // column list follows an alias, never the name).
        let mut into_target = false;
        // The token after an `AS` not followed by `(` is a column label (`ColLabel`
        // admits any keyword), an alias, the first word of a type name, or a nested
        // `WITH` item's `[NOT] MATERIALIZED` — so no arm below reads it: `SELECT 1 AS
        // limit` opens no clause and `… AS from, r` begins no `FROM` list.
        let mut label = false;
        // The two keywords before the current token, for `IS [NOT] DISTINCT FROM`.
        let mut prev: [Option<Keyword>; 2] = [None, None];
        let mut ordinal = 0usize;
        loop {
            if frames.len() == 1 && self.ts.at(&TokenKind::RParen) {
                return Ok(shape);
            }
            if self.ts.at_eof() {
                return Err(self.ts.expected("')'"));
            }
            let tok = self.ts.advance()?;
            ordinal += 1;
            let top = frames.len() == 1;
            match tok.kind {
                TokenKind::LParen | TokenKind::LBracket => {
                    // A parenthesis at a `FROM` item's position is a derived table or a
                    // parenthesized join: its first name is an item too. One that opens a
                    // query or a set operation's operand is no sub-link either; every
                    // other one is an expression's.
                    let from_item = std::mem::take(&mut expect_relation);
                    let set_op = |k: Option<Keyword>| {
                        matches!(
                            k,
                            Some(Keyword::Union | Keyword::Intersect | Keyword::Except)
                        )
                    };
                    let operand = set_op(prev[0])
                        || (matches!(prev[0], Some(Keyword::All | Keyword::Distinct))
                            && set_op(prev[1]));
                    let (sublink, nullable) = frames.last_mut().map_or((false, false), |f| {
                        let nullable = from_item && std::mem::take(&mut f.next_nullable);
                        let expression = !from_item && !operand && f.tokens > 0;
                        (f.sublink || expression, f.nullable || nullable)
                    });
                    frames.push(Frame {
                        sublink,
                        nullable,
                        query: from_item,
                        from_list: from_item,
                        item_start: shape.relations.len(),
                        ..Frame::default()
                    });
                    expect_relation = from_item;
                    prev = [None, prev[0]];
                    continue;
                }
                TokenKind::RParen | TokenKind::RBracket if !top => {
                    expect_relation = false;
                    frames.pop();
                    prev = [None, prev[0]];
                    continue;
                }
                _ => {}
            }
            let Some(frame) = frames.last_mut() else {
                return Err(self.ts.expected("')'"));
            };
            let first = frame.tokens == 0;
            frame.tokens += 1;
            // A keyword is read as the parser reads it at its position, and before `(` the
            // parser reads a TYPE_FUNC_NAME keyword as a function name where an operand
            // starts (`left(t.y, 2)`). That is the only reading `LEFT`, `RIGHT`, `FULL`,
            // `INNER`, `CROSS`, `NATURAL` and `OUTER` have before `(` — the join grammar
            // follows each with `JOIN` or `OUTER`, never `(` — so there they are no keyword
            // here: read as one, a `left(` would leave a join kind pending for the next
            // `JOIN` and mark a relation nullable that no outer join makes so. `JOIN` is
            // join syntax before `(` after a `FROM` item (`t1 JOIN (SELECT …) s`), and
            // `LIKE` / `ILIKE` an operator after an operand (`s LIKE ('a%')`); this scan reads
            // no operand boundary, so those three stay keywords. At an operand start `join(`
            // is a call no catalog resolves, still read here as a join: the statement is
            // refused either way, but this scan's verdict can come first (`42P19` where 18.6
            // answers `42883`).
            let keyword = match tok.kind {
                TokenKind::Keyword(k)
                    if is_call_before_paren(k) && *self.ts.peek_kind() == TokenKind::LParen =>
                {
                    None
                }
                TokenKind::Keyword(k) => Some(k),
                _ => None,
            };
            let ident = |word: &str| {
                matches!(tok.kind, TokenKind::Identifier)
                    && self.ts.text(&tok).eq_ignore_ascii_case(word)
            };

            let into = std::mem::take(&mut into_target);
            if std::mem::take(&mut label) {
                prev = [None, prev[0]];
                continue;
            }
            // `VALUES` is a `ColName` keyword, so it reaches the name test below; followed
            // by `(` it opens a row list (`FROM (VALUES (1), (b)) z`), whose commas
            // separate rows and never begin another item.
            if keyword == Some(Keyword::Values) && *self.ts.peek_kind() == TokenKind::LParen {
                expect_relation = false;
                frame.end_list();
                prev = [keyword, prev[0]];
                continue;
            }
            if std::mem::take(&mut expect_relation) && super::is_colid_name(&tok.kind) {
                let next = self.ts.peek_kind();
                // `ONLY t` and `ONLY (t)`: the name follows.
                if ident("only") && (super::is_colid_name(next) || *next == TokenKind::LParen) {
                    expect_relation = true;
                    prev = [keyword, prev[0]];
                    continue;
                }
                // `ROWS FROM (f(…), …)`: a list of function calls, which its `FROM`
                // re-arms below — `rows` names no relation.
                if ident("rows") && *next == TokenKind::Keyword(Keyword::From) {
                    prev = [keyword, prev[0]];
                    continue;
                }
                let nullable = frame.nullable || std::mem::take(&mut frame.next_nullable);
                let function = *next == TokenKind::LParen && !into;
                if *next != TokenKind::Dot && !function {
                    shape.relations.push(RelationRef {
                        name: self.ts.name(&tok),
                        at: ordinal,
                        sublink: frame.sublink,
                        nullable,
                    });
                }
                prev = [keyword, prev[0]];
                continue;
            }

            if !matches!(
                keyword,
                Some(
                    Keyword::Left | Keyword::Right | Keyword::Full | Keyword::Outer | Keyword::Join
                )
            ) {
                frame.pending_join = None;
            }
            // The call's keyword is read by no arm below (see `keyword`).
            let call_name = keyword.is_none() && matches!(tok.kind, TokenKind::Keyword(_));
            match tok.kind {
                _ if call_name => {}
                TokenKind::Comma => {
                    expect_relation = frame.from_list || frame.locking_of;
                    frame.item_start = shape.relations.len();
                }
                TokenKind::Keyword(Keyword::Select) => {
                    frame.query = true;
                    frame.end_list();
                }
                TokenKind::Keyword(Keyword::Insert) => shape.modifying |= top && first,
                TokenKind::Keyword(Keyword::Delete) => {
                    frame.query = true;
                    frame.delete_using = first;
                    shape.modifying |= top && first;
                }
                TokenKind::Keyword(Keyword::Update) if first => {
                    frame.query = true;
                    expect_relation = true;
                    shape.modifying |= top;
                }
                // `AS (` opens a nested `WITH` item's body or a column definition
                // list, which the parenthesis arm reads.
                TokenKind::Keyword(Keyword::As) => {
                    label = *self.ts.peek_kind() != TokenKind::LParen;
                }
                TokenKind::Keyword(Keyword::Into) => {
                    expect_relation = true;
                    into_target = true;
                }
                // `LATERAL` leads a `FROM` item without being one.
                TokenKind::Keyword(Keyword::Lateral) => expect_relation = true,
                TokenKind::Keyword(Keyword::Left) => frame.pending_join = Some(PendingJoin::Left),
                TokenKind::Keyword(Keyword::Right) => {
                    frame.pending_join = Some(PendingJoin::Right);
                }
                TokenKind::Keyword(Keyword::Full) => frame.pending_join = Some(PendingJoin::Full),
                TokenKind::Keyword(Keyword::Join) => {
                    expect_relation = true;
                    let pending = frame.pending_join.take();
                    if matches!(pending, Some(PendingJoin::Right | PendingJoin::Full)) {
                        for r in &mut shape.relations[frame.item_start..] {
                            r.nullable = true;
                        }
                    }
                    frame.next_nullable =
                        matches!(pending, Some(PendingJoin::Left | PendingJoin::Full));
                }
                TokenKind::Keyword(Keyword::From) => {
                    let predicate = prev[0] == Some(Keyword::Distinct)
                        && matches!(prev[1], Some(Keyword::Is | Keyword::Not));
                    if frame.query && !predicate {
                        expect_relation = true;
                        frame.from_list = true;
                        frame.item_start = shape.relations.len();
                    }
                }
                TokenKind::Keyword(Keyword::Using) => {
                    if std::mem::take(&mut frame.delete_using) {
                        expect_relation = true;
                        frame.from_list = true;
                        frame.item_start = shape.relations.len();
                    }
                }
                TokenKind::Keyword(
                    Keyword::Where
                    | Keyword::Group
                    | Keyword::Having
                    | Keyword::Returning
                    | Keyword::Set,
                ) => frame.end_list(),
                TokenKind::Keyword(Keyword::Order) => {
                    frame.end_list();
                    shape.decorate(top, ordinal, Decoration::OrderBy);
                }
                TokenKind::Keyword(Keyword::Offset) => {
                    frame.end_list();
                    shape.decorate(top, ordinal, Decoration::Offset);
                }
                TokenKind::Keyword(Keyword::Limit) => {
                    frame.end_list();
                    shape.decorate(top, ordinal, Decoration::Limit);
                }
                TokenKind::Keyword(Keyword::For) => {
                    frame.end_list();
                    // `FOR READ ONLY` locks nothing — 18.6's grammar gives it no
                    // locking clause at all.
                    let read_only = matches!(self.ts.peek_kind(), TokenKind::Identifier)
                        && self.ts.peek_text().eq_ignore_ascii_case("read");
                    if read_only {
                        shape.tail(top, ordinal);
                    } else {
                        frame.locking = true;
                        shape.decorate(top, ordinal, Decoration::Locking);
                    }
                }
                TokenKind::Keyword(k @ (Keyword::Union | Keyword::Intersect | Keyword::Except)) => {
                    frame.end_list();
                    frame.query = false;
                    if top {
                        shape.set_ops.push((k, ordinal));
                    }
                }
                TokenKind::Identifier if ident("window") => frame.end_list(),
                TokenKind::Identifier if ident("fetch") => {
                    frame.end_list();
                    shape.decorate(top, ordinal, Decoration::Limit);
                }
                TokenKind::Identifier if frame.locking && ident("of") => {
                    expect_relation = true;
                    frame.locking_of = true;
                }
                _ => {}
            }
            prev = [keyword, prev[0]];
        }
    }
}

/// `TopologicalSort`, over the dependencies the pre-scan read: repeatedly take the first
/// remaining item that depends on no remaining item, **swapping** it into place as 18.6
/// does — so the order among independent items is 18.6's, which decides which item's
/// error a statement reports first. It is not the order 18.6's executor runs
/// data-modifying CTEs in: a referenced one runs when its reader first pulls a row, and
/// the rest run once the statement's own query is done, in the reverse of this order.
/// The statement is therefore refused where that order could be observed
/// (`Parser::wrap_with_ctes`).
///
/// # Errors
///
/// `0A000 mutual recursion between WITH items is not implemented` when no remaining item
/// is free — a cycle of more than one item, a self-reference being no dependency.
fn topological_sort(items: &[ScannedItem<'_>]) -> Result<Vec<usize>> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    let mut depends_on: Vec<Vec<usize>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            (0..items.len())
                .filter(|&j| j != i && item.shape.reads(&items[j].name))
                .collect()
        })
        .collect();
    for i in 0..order.len() {
        let Some(j) = (i..order.len()).find(|&j| depends_on[order[j]].is_empty()) else {
            return Err(Error::RecursiveQueryNotImplemented(
                "mutual recursion between WITH items is not implemented".into(),
            ));
        };
        order.swap(i, j);
        let done = order[i];
        for &k in &order[i + 1..] {
            depends_on[k].retain(|&d| d != done);
        }
    }
    Ok(order)
}

impl<'a> Parser<'a> {
    /// The prefix of `checkWellFormedRecursion` that needs only the raw tree, for one
    /// item: a self-referencing body must be a `SELECT` whose top level is a `UNION`,
    /// with no `ORDER BY` / `OFFSET` / `LIMIT` / locking clause over it, no
    /// self-reference in its left arm and at most one in its right, none in a sub-link
    /// and none on an outer join's nullable side. What needs the planned tree — the
    /// `INTERSECT` / `EXCEPT` contexts of a set operation inside the recursive term — is
    /// the body parse's (`check_recursive_placement`).
    ///
    /// 18.6's raw parse runs before all of it, so the clauses over the whole query are
    /// first read through their grammar ([`Self::parse_query_tail`]), and a syntax error
    /// there is the verdict.
    ///
    /// # Errors
    ///
    /// `42601` for a malformed clause over the whole query; `42P19` for the form and the
    /// self-reference counts and `0A000` for the decorations, each with 18.6's
    /// sentence.
    fn check_well_formed_recursion(&mut self, item: &ScannedItem<'a>) -> Result<()> {
        let (name, shape) = (&item.name, &item.shape);
        if !shape.reads(name) {
            return Ok(());
        }
        if shape.modifying {
            return Err(Error::InvalidRecursion(format!(
                "recursive query \"{name}\" must not contain data-modifying statements"
            )));
        }
        // A body leading with a parenthesized query that no set operation follows merges
        // that query's clauses with the ones after it (`QueryClauses::merge`). Before a set
        // operation the parenthesized query is the non-recursive term, whose own parse
        // refuses a merge inside it — reached behind any verdict here by the syntax-error
        // re-parse (`Self::first_syntax_error`).
        let inner = if shape.leads_with_paren && shape.set_ops.is_empty() {
            self.ts.restore(item.body.clone());
            self.parse_carried_clauses()?
        } else {
            QueryClauses::default()
        };
        if let Some(at) = shape.tail_at {
            self.ts.restore(item.body.clone());
            for _ in 1..at {
                self.ts.bump()?;
            }
            self.parse_query_tail(inner)?;
        }
        let split = match shape.top_set_op() {
            Some((Keyword::Union, split)) => split,
            // A parenthesized body with no set operator outside the parentheses: its top
            // level is not in view here, so its parse decides.
            None if shape.leads_with_paren => return Ok(()),
            _ => return Err(super::select::invalid_recursion_form(name)),
        };
        if let Some(decoration) = shape.decoration {
            return Err(recursive_decoration_refusal(decoration.spelling()));
        }
        // `checkWellFormedRecursionWalker` over the left arm, then the right, in order: a
        // sub-link overrides the arm's own context, an outer join's nullable side overrides
        // the right arm's, and the right arm admits one reference.
        let mut in_right_arm = 0usize;
        for r in shape.relations.iter().filter(|r| r.name == *name) {
            let place = if r.sublink {
                "within a subquery"
            } else if r.at < split {
                "within its non-recursive term"
            } else if r.nullable {
                "within an outer join"
            } else {
                in_right_arm += 1;
                if in_right_arm == 1 {
                    continue;
                }
                "more than once"
            };
            return Err(Error::InvalidRecursion(format!(
                "recursive reference to query \"{name}\" must not appear {place}"
            )));
        }
        Ok(())
    }
}

/// 18.6's refusal of a decoration over a whole recursive query — `ORDER BY`, `OFFSET`,
/// `LIMIT` (and `FETCH`, its other spelling), `FOR UPDATE/SHARE`.
fn recursive_decoration_refusal(what: &str) -> Error {
    Error::RecursiveQueryNotImplemented(format!("{what} in a recursive query is not implemented"))
}

/// What the clauses over a whole query showed while they were read.
#[derive(Default)]
pub(super) struct Tail {
    /// The decoration `checkWellFormedRecursion` tests first among them.
    first: Option<Decoration>,
    /// Which of them the query carries.
    clauses: QueryClauses,
}

impl Tail {
    /// Note a decoration, keeping the one `checkWellFormedRecursion` tests first.
    fn note(&mut self, decoration: Decoration) {
        self.first = Some(self.first.map_or(decoration, |d| d.min(decoration)));
        match decoration {
            Decoration::OrderBy => self.clauses.order_by = true,
            Decoration::Offset => self.clauses.offset = true,
            Decoration::Limit => self.clauses.limit = true,
            Decoration::Locking => {}
        }
    }

    /// 18.6's refusal of the decoration it tests first, if the query carries one — a
    /// `FOR READ ONLY` alone locks nothing.
    pub(super) fn refusal(&self) -> Option<Error> {
        self.first
            .map(|d| recursive_decoration_refusal(d.spelling()))
    }
}

impl Parser<'_> {
    /// Whether the cursor is at a clause over a whole query: `ORDER BY`, `OFFSET`,
    /// `LIMIT`, `FETCH` or `FOR`.
    pub(super) fn at_query_tail(&self) -> bool {
        self.ts.at_kw(Keyword::Order)
            || self.ts.at_kw(Keyword::Offset)
            || self.ts.at_kw(Keyword::Limit)
            || self.ts.at_kw(Keyword::For)
            || self.at_ident("fetch")
    }

    /// Read the clauses over a whole query, from the cursor to (not past) the body's
    /// `)`, through 18.6's grammar for them (`select_no_parens`): an optional
    /// `sort_clause`, then a `select_limit` and a `for_locking_clause` in either order,
    /// each at most once — so a second `ORDER BY` / `LIMIT`, an `ORDER BY` after either,
    /// `LIMIT #,#`, a `FETCH` count or an `OFFSET … ROWS` value that is no `c_expr` or
    /// signed constant, and anything left before the `)` is a syntax error, as the raw
    /// parse reports it ahead of every verdict. Then the clauses are merged into
    /// `inner` — the clauses of the parenthesized query they follow, with no set
    /// operation between, or none — as `insertSelectOptions` merges them as the grammar
    /// builds the statement, `42601` too ([`QueryClauses::merge`]). Returns what was
    /// read, the merged clauses among it.
    ///
    /// A clause's expressions are parsed by the ordinary parser, which also analyses a
    /// subquery among them. An analysis error there is no grammar verdict — 18.6 has
    /// refused the decoration before analysing any of it — so it ends the reading, and
    /// the clauses read up to it decide the refusal; a `42601` 18.6's grammar would
    /// raise from the unread rest, or from `insertSelectOptions`, is not raised.
    ///
    /// # Errors
    ///
    /// `42601`: the syntax errors and `insertSelectOptions` refusals above, and also clause
    /// text 18.6's grammar reads and pglet's expression parser does not — among it a
    /// `USING OPERATOR(…)` sort operator ([`Self::parse_sort_key`]), where 18.6 goes on
    /// to refuse the decoration.
    pub(super) fn parse_query_tail(&mut self, inner: QueryClauses) -> Result<Tail> {
        let wait_policy = self.lock_wait_policy_refusal.take();
        self.saw_skip_locked = false;
        let mut tail = Tail::default();
        let read = self.parse_query_tail_clauses(&mut tail);
        self.lock_wait_policy_refusal = wait_policy;
        tail.clauses.skip_locked = self.saw_skip_locked;
        match read {
            Err(err @ Error::Syntax { .. }) => return Err(err),
            Ok(()) if !self.ts.at(&TokenKind::RParen) => {
                return Err(self.ts.syntax_error_at_or_near());
            }
            Ok(()) => {
                tail.clauses = inner
                    .merge(tail.clauses)
                    .map_err(|e| self.before_analysis(e))?;
            }
            // an analysis error ends the reading
            Err(_) => {}
        }
        Ok(tail)
    }

    /// The clauses the parenthesized query at the cursor carries (`(` next): the ones
    /// over its whole query, merged into those of a parenthesized query it leads with
    /// and no set operation follows — each merge refused as [`QueryClauses::merge`]
    /// refuses it, innermost first, as the grammar builds them. A parenthesized query a
    /// set operation follows carries nothing into the clauses after the set operation.
    ///
    /// # Errors
    ///
    /// `42601` as [`Self::parse_query_tail`] raises it.
    fn parse_carried_clauses(&mut self) -> Result<QueryClauses> {
        self.ts.bump()?;
        let start = self.ts.checkpoint();
        let shape = self.scan_with_body()?;
        let inner = if shape.leads_with_paren && shape.set_ops.is_empty() {
            self.ts.restore(start.clone());
            // One nesting level a parenthesis, as the body's own parse charges it.
            self.enter()?;
            let inner = self.parse_carried_clauses();
            self.leave();
            inner?
        } else {
            QueryClauses::default()
        };
        let Some(at) = shape.tail_at else {
            return Ok(inner);
        };
        self.ts.restore(start);
        for _ in 1..at {
            self.ts.bump()?;
        }
        Ok(self.parse_query_tail(inner)?.clauses)
    }

    /// Each clause is noted as it is entered, so an analysis error inside it still
    /// leaves it counted.
    fn parse_query_tail_clauses(&mut self, tail: &mut Tail) -> Result<()> {
        if self.ts.eat_kw(Keyword::Order)? {
            tail.note(Decoration::OrderBy);
            self.ts.expect_kw(Keyword::By)?;
            self.comma_list(Self::parse_sort_key)?;
        }
        if self.ts.at_kw(Keyword::For) {
            self.parse_locking_tail(tail)?;
            self.parse_select_limit(tail)
        } else {
            self.parse_select_limit(tail)?;
            self.parse_locking_tail(tail)
        }
    }

    /// One `sortby`: `a_expr [ASC | DESC | USING operator] [NULLS FIRST | LAST]`, the
    /// operator any one the lexer reads. The grammar's `OPERATOR([schema.]op)` spelling
    /// is refused here as a syntax error, where 18.6 reaches its decoration refusal.
    fn parse_sort_key(&mut self) -> Result<()> {
        self.parse_expr_bp(0)?;
        if self.ts.eat_kw(Keyword::Using)? {
            let op = self.ts.advance()?;
            if !matches!(
                op.kind,
                TokenKind::Containment
                    | TokenKind::ContainedBy
                    | TokenKind::Arrow
                    | TokenKind::ArrowText
                    | TokenKind::HashArrow
                    | TokenKind::HashArrowText
                    | TokenKind::Question
                    | TokenKind::QuestionPipe
                    | TokenKind::QuestionAmp
                    | TokenKind::Concat
                    | TokenKind::Eq
                    | TokenKind::NotEq
                    | TokenKind::Lt
                    | TokenKind::Le
                    | TokenKind::Gt
                    | TokenKind::Ge
                    | TokenKind::Plus
                    | TokenKind::Minus
                    | TokenKind::Star
                    | TokenKind::Slash
                    | TokenKind::Percent
                    | TokenKind::Caret
            ) {
                return Err(self.ts.expected("an operator"));
            }
        } else if !self.ts.eat_kw(Keyword::Asc)? {
            self.ts.eat_kw(Keyword::Desc)?;
        }
        if self.ts.eat_kw(Keyword::Nulls)? && !self.ts.eat_kw(Keyword::First)? {
            self.ts.expect_kw(Keyword::Last)?;
        }
        Ok(())
    }

    /// An optional `for_locking_clause`, noting a locking one (`FOR READ ONLY` is none).
    fn parse_locking_tail(&mut self, tail: &mut Tail) -> Result<()> {
        if !self.ts.at_kw(Keyword::For) {
            return Ok(());
        }
        let read_only = matches!(self.ts.peek2_kind(), TokenKind::Identifier)
            && self.ts.peek2_text().eq_ignore_ascii_case("read");
        if !read_only {
            tail.note(Decoration::Locking);
        }
        self.parse_for_locking()
    }

    /// An optional `select_limit`: a `limit_clause` and an `offset_clause`, in either
    /// order, each at most once.
    fn parse_select_limit(&mut self, tail: &mut Tail) -> Result<()> {
        if self.ts.at_kw(Keyword::Offset) {
            self.parse_offset_clause(tail)?;
            self.parse_limit_clause(tail)
        } else {
            self.parse_limit_clause(tail)?;
            self.parse_offset_clause(tail)
        }
    }

    /// An optional `limit_clause`: `LIMIT { a_expr | ALL }`, or
    /// `FETCH { FIRST | NEXT } [value] { ROW | ROWS } { ONLY | WITH TIES }`.
    fn parse_limit_clause(&mut self, tail: &mut Tail) -> Result<()> {
        if self.ts.eat_kw(Keyword::Limit)? {
            tail.note(Decoration::Limit);
            if !self.ts.eat_kw(Keyword::All)? {
                self.parse_expr_bp(0)?;
            }
            if self.ts.at(&TokenKind::Comma) {
                // `gram.y`'s `limit_clause: LIMIT select_limit_value ',' select_offset_value`
                return Err(self.before_analysis(Error::Syntax {
                    message: "LIMIT #,# syntax is not supported".into(),
                    position: None,
                }));
            }
        } else if self.eat_ident("fetch")? {
            tail.note(Decoration::Limit);
            if !self.ts.eat_kw(Keyword::First)? {
                self.expect_ident("next")?;
            }
            if !(self.at_ident("row") || self.at_ident("rows"))
                && self.parse_fetch_first_value()?.is_none()
            {
                // at the operand a sign takes, which is where 18.6's parse stops
                if !self.ts.at_kw(Keyword::Not) {
                    self.ts.bump()?;
                }
                return Err(self.ts.syntax_error_at_or_near());
            }
            if !self.eat_ident("rows")? {
                self.expect_ident("row")?;
            }
            if !self.eat_ident("only")? {
                self.ts.expect_kw(Keyword::With)?;
                self.expect_ident("ties")?;
                tail.clauses.with_ties = true;
            }
        }
        Ok(())
    }

    /// An optional `offset_clause`: `OFFSET a_expr`, or `OFFSET value { ROW | ROWS }`
    /// with the value [`Self::parse_fetch_first_value`] reads.
    fn parse_offset_clause(&mut self, tail: &mut Tail) -> Result<()> {
        if self.ts.eat_kw(Keyword::Offset)? {
            tail.note(Decoration::Offset);
            match self.parse_fetch_first_value()? {
                None => {
                    self.parse_expr_bp(0)?;
                }
                Some(value) => {
                    if !(self.eat_ident("rows")? || self.eat_ident("row")?) {
                        self.continue_expr_bp(value, 0)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A `select_fetch_first_value` — a `c_expr`, or a numeric constant after a sign —
    /// or `None`, with nothing read, at a `+` / `-` no number follows or at a `NOT`,
    /// which only a full expression begins.
    fn parse_fetch_first_value(&mut self) -> Result<Option<crate::planner::Expr>> {
        if self.ts.at_kw(Keyword::Not) {
            return Ok(None);
        }
        if self.ts.at(&TokenKind::Plus) || self.ts.at(&TokenKind::Minus) {
            if *self.ts.peek2_kind() != TokenKind::Number {
                return Ok(None);
            }
            self.ts.bump()?;
        }
        self.parse_primary().map(Some)
    }
}

/// Whether `kw`, followed by `(`, can only be a function name — a TYPE_FUNC_NAME keyword
/// whose other readings (a join kind, `OUTER`, a postfix or infix test) are never followed
/// by `(`. `JOIN` (a derived table after it), `LIKE` and `ILIKE` (a parenthesized pattern)
/// are not among them.
fn is_call_before_paren(kw: Keyword) -> bool {
    kw.pg_class() == crate::lexer::PgClass::TypeFuncName
        && !matches!(kw, Keyword::Join | Keyword::Like | Keyword::Ilike)
}

#[cfg(test)]
mod tests {
    use crate::catalog::Schema;
    use crate::parser::parse;
    use crate::planner::{CteBody, Plan};

    fn plan(sql: &str, schema: &Schema) -> Plan {
        parse(sql, schema).expect("statement parses").remove(0)
    }

    fn entry_names(plan: &Plan) -> Vec<&str> {
        let Plan::With { ctes, .. } = plan else {
            panic!("expected a Plan::With, got {plan:?}");
        };
        ctes.iter().map(|c| c.name.as_str()).collect()
    }

    /// Several recursive CTEs and no write keep the statement on the lock-free read
    /// path: the `Plan::With` is read-only because every entry and the body are.
    #[test]
    fn recursive_entries_alone_keep_the_statement_read_only() {
        let schema = Schema::new();
        let p = plan(
            "WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 3), \
             b(m) AS (SELECT n FROM a UNION ALL SELECT m + 1 FROM b WHERE m < 5) \
             SELECT m FROM b",
            &schema,
        );
        assert_eq!(entry_names(&p), ["a", "b"]);
        assert!(
            p.is_read_only(),
            "a recursive-only WITH must stay read-only"
        );
        assert!(!p.mutates_storage());
    }

    /// A data-modifying entry makes the same statement a write.
    #[test]
    fn a_data_modifying_entry_makes_the_statement_a_write() {
        let mut schema = Schema::new();
        let create = plan("CREATE TABLE s (x INT)", &schema);
        crate::executor::execute(&create, &mut schema).expect("table created");
        let p = plan(
            "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3), \
             w AS (INSERT INTO s SELECT n FROM r RETURNING x) SELECT count(*) FROM w",
            &schema,
        );
        let Plan::With { ctes, .. } = &p else {
            panic!("expected a Plan::With");
        };
        assert!(matches!(ctes[0].body, CteBody::Recursive { .. }));
        assert!(matches!(ctes[1].body, CteBody::Modifying { .. }));
        assert!(!p.is_read_only());
        assert!(p.mutates_storage());
    }

    /// Recursive entries alone are read-only, so whether the statement writes is the
    /// body's to say: a body that writes makes it a write, on both predicates.
    #[test]
    fn a_writing_body_makes_a_recursive_with_a_write() {
        let mut schema = Schema::new();
        let create = plan("CREATE TABLE s (x INT)", &schema);
        crate::executor::execute(&create, &mut schema).expect("table created");
        let p = plan(
            "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3) \
             INSERT INTO s SELECT n FROM r",
            &schema,
        );
        let Plan::With { ctes, .. } = &p else {
            panic!("expected a Plan::With, got {p:?}");
        };
        assert!(matches!(ctes[0].body, CteBody::Recursive { .. }));
        assert!(!p.is_read_only(), "a writing body is no read");
        assert!(p.mutates_storage(), "a writing body mutates storage");
    }

    /// The entries run in `TopologicalSort`'s order, which **swaps** the first free item
    /// into place: over `p`, `q`, `s` with `p` and `q` both reading `s` that is `s`, `q`,
    /// `p` — where taking the earliest-declared free item each round gives `s`, `p`, `q`.
    #[test]
    fn entries_run_in_topological_sort_order() {
        let schema = Schema::new();
        let p = plan(
            "WITH RECURSIVE \
             p(n) AS (SELECT k FROM s UNION ALL SELECT n FROM p WHERE false), \
             q(n) AS (SELECT k FROM s UNION ALL SELECT n FROM q WHERE false), \
             s(k) AS (SELECT 1 UNION ALL SELECT k FROM s WHERE false) \
             SELECT 1",
            &schema,
        );
        assert_eq!(entry_names(&p), ["s", "q", "p"]);
    }

    /// A cycle of more than one item is 18.6's own refusal, message and all.
    #[test]
    fn a_cycle_is_mutual_recursion() {
        let err = parse(
            "WITH RECURSIVE a AS (SELECT * FROM b), b AS (SELECT * FROM a) SELECT 1",
            &Schema::new(),
        )
        .expect_err("a cycle is refused");
        assert_eq!(err.sqlstate(), "0A000");
        assert_eq!(
            err.to_string(),
            "mutual recursion between WITH items is not implemented"
        );
    }
}
