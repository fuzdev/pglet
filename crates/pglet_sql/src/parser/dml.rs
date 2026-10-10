//! DML statement parsing (INSERT), fused into the planner [`Plan`] IR.
//!
//! `INSERT INTO t [(cols)] {VALUES …|SELECT …|DEFAULT VALUES} [ON CONFLICT …] [RETURNING …]`,
//! lowered through the shared [`lower_insert`], which applies the column-mapping,
//! `DEFAULT`-fill, and coercion. The `ON CONFLICT` grammar parses the
//! **expression / partial** arbiter target (`ON CONFLICT (a, COALESCE(b, ''))
//! WHERE …`) that a bare-column-only conflict target cannot represent — the
//! conformance gap this parser closes, the spine upsert it exists to unblock —
//! plus the `DO UPDATE … WHERE` filter.
//!
//! Every clause is parsed to raw IR and handed to the `lower_*` helpers **in
//! Postgres's own transform order**; this module decides nothing about types. The
//! `ON CONFLICT` action in particular is resolved and gated by its `lower_*` step, not
//! here, because it must follow the `VALUES` cells.

use crate::lexer::{Keyword, TokenKind};
use crate::planner::{
    ConflictTarget, Expr, InsertRowSource, OnConflictAction, OnConflictClause, Overriding, Plan,
    lower_delete, lower_insert, lower_update,
};
use crate::{Error, Result};

use super::Parser;

impl Parser<'_> {
    /// Parse an `INSERT` statement into a [`Plan::Insert`]. Depth-guarded (a row
    /// source may embed a SELECT / subqueries).
    pub(super) fn parse_insert(&mut self) -> Result<Plan> {
        self.enter()?;
        let result = self.parse_insert_inner();
        self.leave();
        result
    }

    fn parse_insert_inner(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Insert)?;
        self.ts.expect_kw(Keyword::Into)?;
        let table = self.parse_relation_name()?;
        self.touched_tables.push(table.clone());

        // Optional explicit column list `(a, b, c)`. A leading `(` here is always
        // the column list — the row source (`VALUES`/`SELECT`) starts with a keyword.
        let columns = if self.ts.at(&TokenKind::LParen) {
            self.parse_paren_name_list()?
        } else {
            Vec::new()
        };

        // Optional `OVERRIDING { SYSTEM | USER } VALUE`, between the column list and
        // the row source (all contextual identifiers, unreserved in Postgres).
        let overriding = self.parse_overriding()?;

        // Row source: literal `VALUES` tuples, an `INSERT … SELECT` sub-plan, or the
        // row-less `DEFAULT VALUES`.
        //
        // `DEFAULT VALUES` is its **own** `insert_rest` alternative in `gram.y`, and the
        // alternative carries neither a column list nor `OVERRIDING` — so both of those
        // spellings are a syntax error at the `DEFAULT` token rather than a statement
        // this parser then has to give a meaning to. Measured on 18.6: `INSERT INTO t (a)
        // DEFAULT VALUES` and `INSERT INTO t OVERRIDING SYSTEM VALUE DEFAULT VALUES` are
        // both `42601 syntax error at or near "DEFAULT"`. Guarding here rather than after
        // the fact is what keeps the position right: the token is unconsumed, so the
        // fall-through's `expected` names `DEFAULT`, where a later arity check would have
        // named the row instead (`INSERT has more target columns than expressions`).
        //
        // The lowering is one empty row. `lower_insert_rows` widens every `VALUES` tuple
        // to table width with `Expr::Default`, so a zero-cell tuple *is* the all-defaults
        // row — the same `Plan::Insert` a `VALUES (DEFAULT, …)` twin produces, which is
        // why `ON CONFLICT`, `RETURNING` and the correlated re-walk need nothing new.
        let source = if self.ts.eat_kw(Keyword::Values)? {
            InsertRowSource::Values(self.parse_values_rows()?)
        } else if self.ts.at_kw(Keyword::Default)
            && columns.is_empty()
            && overriding == Overriding::None
        {
            self.ts.bump()?;
            self.ts.expect_kw(Keyword::Values)?;
            InsertRowSource::Values(vec![Vec::new()])
        } else if self.ts.at_kw(Keyword::Select) {
            InsertRowSource::Select(Box::new(self.parse_select()?))
        } else {
            return Err(self.ts.expected("VALUES, SELECT, or DEFAULT VALUES"));
        };

        let on_conflict = if self.at_on_conflict() {
            Some(self.parse_on_conflict()?)
        } else {
            None
        };

        let returning = if self.ts.eat_kw(Keyword::Returning)? {
            self.parse_projection()?
        } else {
            Vec::new()
        };

        lower_insert(
            table,
            &columns,
            source,
            returning,
            on_conflict,
            overriding,
            self.schema,
        )
    }

    /// Parse an optional `OVERRIDING { SYSTEM | USER } VALUE` clause (`Overriding::None`
    /// if absent). `OVERRIDING`/`SYSTEM`/`USER`/`VALUE` are contextual identifiers, so
    /// they stay usable as ordinary names elsewhere.
    fn parse_overriding(&mut self) -> Result<Overriding> {
        if !self.eat_ident("overriding")? {
            return Ok(Overriding::None);
        }
        let overriding = if self.eat_ident("system")? {
            Overriding::System
        } else if self.eat_ident("user")? {
            Overriding::User
        } else {
            return Err(self.ts.expected("SYSTEM or USER"));
        };
        self.expect_ident("value")?;
        Ok(overriding)
    }

    /// Parse an `UPDATE` statement into a [`Plan::Update`]. Depth-guarded (an
    /// assignment value or `WHERE` may embed subqueries).
    pub(super) fn parse_update(&mut self) -> Result<Plan> {
        self.enter()?;
        let result = self.parse_update_inner();
        self.leave();
        result
    }

    fn parse_update_inner(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Update)?;
        let table = self.parse_relation_name()?;
        self.touched_tables.push(table.clone());
        // An optional target alias (`UPDATE t AS o` / `UPDATE t o`). `SET` is a
        // keyword token, so an unaliased `UPDATE t SET …` parses no alias here.
        let alias = self.parse_table_alias()?;
        self.ts.expect_kw(Keyword::Set)?;

        // `col = expr [, …]` — a bare column name before each `=`. 18.6's grammar also
        // takes a multiple-column, subscripted or dotted target, which this refuses
        // `42601` (`EDGES.md` row 245). The shared `lower_update` coerces each value to
        // the column type.
        let assignments = self.comma_list(|p| {
            let col = p.expect_name("a column name")?;
            p.ts.expect(&TokenKind::Eq)?;
            let expr = p.parse_expr_bp(0)?;
            Ok((col, expr))
        })?;

        // Optional `FROM <table>` joined to the target. A single source covers the
        // spine's `UPDATE … SET … FROM actor a WHERE …`; a comma-separated from-list
        // (cross join) is not supported and fails loud.
        let from = if self.ts.eat_kw(Keyword::From)? {
            let (plan, _) = self.parse_table_factor()?;
            if self.ts.at(&TokenKind::Comma) {
                return Err(Error::NotImplemented(
                    "UPDATE ... FROM with multiple source tables not supported".into(),
                ));
            }
            Some(plan)
        } else {
            None
        };

        let filter = if self.ts.eat_kw(Keyword::Where)? {
            Some(self.parse_expr_bp(0)?)
        } else {
            None
        };
        let returning = if self.ts.eat_kw(Keyword::Returning)? {
            self.parse_projection()?
        } else {
            Vec::new()
        };

        lower_update(
            table,
            alias,
            from,
            assignments,
            filter,
            returning,
            self.schema,
        )
    }

    /// Parse a `DELETE FROM` statement into a [`Plan::Delete`]. Depth-guarded (the
    /// `WHERE` may embed subqueries).
    pub(super) fn parse_delete(&mut self) -> Result<Plan> {
        self.enter()?;
        let result = self.parse_delete_inner();
        self.leave();
        result
    }

    fn parse_delete_inner(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Delete)?;
        self.ts.expect_kw(Keyword::From)?;
        let table = self.parse_relation_name()?;
        self.touched_tables.push(table.clone());

        let filter = if self.ts.eat_kw(Keyword::Where)? {
            Some(self.parse_expr_bp(0)?)
        } else {
            None
        };
        let returning = if self.ts.eat_kw(Keyword::Returning)? {
            self.parse_projection()?
        } else {
            Vec::new()
        };

        lower_delete(table, filter, returning, self.schema)
    }

    /// Parse a (possibly schema-qualified) object name, reduced to a single table
    /// name under pglet's one implicit `public` schema (see
    /// [`super::reduce_qualified_name`]): `public.t` → `t`, any other schema
    /// qualifier or 3+ dotted parts fails loud instead of silently dropping the
    /// prefix.
    ///
    /// For a position `gram.y` reads as an `any_name` (`CREATE TYPE`, `DROP TABLE` /
    /// `INDEX` / `SEQUENCE`) or a `Typename` (`DROP TYPE`): 18.6 takes any number of
    /// parts there and refuses a four-part one only when the statement **runs**, so the
    /// refusal is not recorded as outranking a too-deep chain. A position that reads a
    /// `qualified_name` takes [`Self::parse_relation_name`] instead.
    pub(super) fn parse_object_name(&mut self) -> Result<String> {
        let parts = self.parse_dotted_name()?;
        super::reduce_qualified_name(&parts)
    }

    /// [`Self::parse_object_name`] for a position whose name 18.6's **raw parse**
    /// refuses at four parts ([`Self::reduce_grammar_name`]): a `qualified_name` or a
    /// `relation_expr` — a statement's target relation, a `REFERENCES` table, a
    /// `TRUNCATE` / `COPY` / `CREATE INDEX … ON` table, a sequence's name — whose
    /// action is `makeRangeVarFromQualifiedName`; and a position `gram.y` reads as a
    /// single `ColId` (a `CREATE SCHEMA` / `DROP SCHEMA` / `CREATE INDEX` name), where
    /// any dotted name is the grammar's own `42601` — so the four-part refusal is one
    /// 18.6 raises too, at the raw parse, though 18.6 raises it at two parts as well.
    pub(super) fn parse_relation_name(&mut self) -> Result<String> {
        let parts = self.parse_dotted_name()?;
        self.reduce_grammar_name(&parts)
    }

    /// Read `name [. name …]`, the parts unreduced.
    pub(super) fn parse_dotted_name(&mut self) -> Result<Vec<String>> {
        let mut parts = vec![self.expect_name("a name")?];
        while self.ts.at(&TokenKind::Dot) {
            self.ts.bump()?;
            parts.push(self.expect_name("a name after '.'")?);
        }
        Ok(parts)
    }

    /// Consume one identifier as a name, or a `42601` error. Accepts a bare or
    /// quoted identifier, or a **non-reserved keyword** used as an identifier (e.g.
    /// a column named `sequence`); see [`super::is_reserved`].
    ///
    /// The name is read through [`TokenStream::name`](super::stream::TokenStream::name),
    /// so an unquoted spelling is downcased and a quoted one is exact — Postgres's
    /// `ColId` rule.
    pub(super) fn expect_name(&mut self, what: &str) -> Result<String> {
        let tok = self.ts.advance()?;
        if super::is_colid_name(&tok.kind) {
            Ok(self.ts.name(&tok))
        } else {
            Err(self.ts.expected(what))
        }
    }

    /// Parse a parenthesized comma-separated identifier list `( a, b, c )`.
    pub(super) fn parse_paren_name_list(&mut self) -> Result<Vec<String>> {
        self.ts.expect(&TokenKind::LParen)?;
        let names = self.comma_list(|p| p.expect_name("a column name"))?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(names)
    }

    /// Parse one or more `VALUES` tuples `(e, …), (e, …)`. A `DEFAULT` keyword in a
    /// value position lowers to [`Expr::Default`] (the executor resolves the catalog
    /// default).
    pub(super) fn parse_values_rows(&mut self) -> Result<Vec<Vec<Expr>>> {
        self.comma_list(|p| {
            p.ts.expect(&TokenKind::LParen)?;
            let row = p.comma_list(|q| {
                if q.ts.eat_kw(Keyword::Default)? {
                    Ok(Expr::Default)
                } else {
                    q.parse_expr_bp(0)
                }
            })?;
            p.ts.expect(&TokenKind::RParen)?;
            Ok(row)
        })
    }

    /// Whether an `ON CONFLICT` clause begins at the cursor (`ON` then `CONFLICT`,
    /// distinguishing it from an `ON` that would start a JOIN).
    fn at_on_conflict(&self) -> bool {
        self.ts.at_kw(Keyword::On)
            && matches!(self.ts.peek2_kind(), TokenKind::Keyword(Keyword::Conflict))
    }

    /// Parse an `ON CONFLICT [ ( key, … ) [WHERE pred] ] DO {NOTHING | UPDATE SET …
    /// [WHERE pred]}` clause into **raw** lowered IR.
    ///
    /// The arbiter target keys are full expressions (a bare column lowers to
    /// `Expr::Column`, an expression part like `COALESCE(scope_kind, '')` to its IR),
    /// and an optional partial-index `WHERE` predicate — the shape the executor
    /// matches against the table's unique indexes.
    ///
    /// Nothing here is resolved, gated, or coerced: that is the planner's
    /// `lower_on_conflict`, and it has to run *after* the row source.
    /// `transformInsertStmt` transforms the `VALUES` source, then
    /// `transformOnConflictClause`, then the `RETURNING` clause — so when two of those
    /// clauses are ill-typed Postgres reports the earlier one, and resolving the action
    /// at parse time (before the source had been lowered at all) reported it last.
    fn parse_on_conflict(&mut self) -> Result<OnConflictClause> {
        self.ts.expect_kw(Keyword::On)?;
        self.ts.expect_kw(Keyword::Conflict)?;

        // Optional arbiter target: `( key, key, … ) [WHERE predicate]`. Absent =
        // target-less (Postgres permits only `DO NOTHING`; the executor arbitrates
        // against every unique constraint).
        let target = if self.ts.at(&TokenKind::LParen) {
            self.ts.bump()?;
            let keys = self.comma_list(|p| p.parse_expr_bp(0))?;
            self.ts.expect(&TokenKind::RParen)?;
            let predicate = if self.ts.eat_kw(Keyword::Where)? {
                Some(self.parse_expr_bp(0)?)
            } else {
                None
            };
            Some(ConflictTarget { keys, predicate })
        } else {
            None
        };

        self.ts.expect_kw(Keyword::Do)?;

        let action = if self.ts.eat_kw(Keyword::Nothing)? {
            OnConflictAction::DoNothing
        } else if self.ts.eat_kw(Keyword::Update)? {
            self.ts.expect_kw(Keyword::Set)?;
            let assignments = self.comma_list(|p| {
                let col = p.expect_name("a column name")?;
                p.ts.expect(&TokenKind::Eq)?;
                let expr = p.parse_expr_bp(0)?;
                Ok((col, expr))
            })?;
            let where_predicate = if self.ts.eat_kw(Keyword::Where)? {
                Some(self.parse_expr_bp(0)?)
            } else {
                None
            };
            OnConflictAction::DoUpdate {
                assignments,
                where_predicate,
            }
        } else {
            return Err(self.ts.expected("NOTHING or UPDATE"));
        };

        Ok(OnConflictClause { target, action })
    }
}
