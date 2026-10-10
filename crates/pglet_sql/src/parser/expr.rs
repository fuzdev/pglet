//! Pratt-core expression parsing, fused straight into the planner [`Expr`] IR.
//!
//! A single [`Parser::parse_expr_bp`] loop drives a **pure** binary-operator
//! binding-power table ([`infix_bp`]); the keyword-led forms that aren't clean
//! binary operators — `NOT` (prefix), unary `-`/`+`, `::` cast, the `IS …`
//! postfix family — are explicit productions around the loop. The binding-power
//! ladder follows modern Postgres precedence (loosest → tightest): `OR` < `AND`
//! < `NOT` < `IS` < comparison < the "any other operator" band
//! (`@> <@ -> ->> #> #>> ? ?| ?&`) < additive < multiplicative (`* / %`) <
//! exponentiation (`^`) < `AT TIME ZONE` < unary `-`/`+` < `::`. `^` is
//! left-associative and binds tighter than `* / %`;
//! unary minus binds tighter than `^` (`-2^2` is `(-2)^2`), all matching Postgres.

use crate::lexer::{Keyword, PgClass, Token, TokenKind};
use crate::planner::{
    BinaryOp, BoolTest, Expr, Literal, SetQuantifier, SqlValueFunction, StringFunc, WindowFunc,
};
use crate::types::{PgInterval, TypeOid, Value};
use crate::{Error, Result};

use super::Parser;

// Binding powers, loosest → tightest. Left-associative infix operators use
// `(bp, bp + 1)` so a same-precedence operator on the right doesn't re-bind.
const BP_OR: u8 = 2;
const BP_AND: u8 = 4;
/// The minimum binding power a `NOT`-prefixed operand parses at — above `AND` so
/// `NOT a AND b` is `(NOT a) AND b`, below comparison/`IS` so `NOT a = b` is
/// `NOT (a = b)`.
const BP_NOT: u8 = 7;
const BP_IS: u8 = 8;
const BP_CMP: u8 = 10;
/// The `BETWEEN`/`IN`/`LIKE`/`ILIKE` predicate band — tighter than comparison,
/// looser than the "any other operator" band.
const BP_PRED: u8 = 12;
/// Postgres's "any other operator" band — `@> <@ -> ->> #> #>> ? ?| ?&`. Tighter
/// than comparison, looser than additive.
const BP_OTHER: u8 = 14;
const BP_ADD: u8 = 16;
const BP_MUL: u8 = 18;
/// Exponentiation `^` — tighter than `* / %` (`BP_MUL`), looser than `AT TIME
/// ZONE` (`BP_AT`), matching Postgres. Left-associative (`2^3^2` is `(2^3)^2`).
const BP_POW: u8 = 19;
/// `AT TIME ZONE` — a high-binding postfix shift.
const BP_AT: u8 = 20;
/// Unary `-`/`+` bind tighter than every binary operator.
const BP_UNARY: u8 = 22;
/// `::` cast binds tightest of all (postfix).
const BP_CAST: u8 = 26;

/// The pure binary-operator binding-power table: `(left_bp, right_bp, op)` for an
/// infix operator token, or `None` if the token isn't one.
///
/// Every entry is left-associative (`(bp, bp + 1)`), including `%` (modulo, at the
/// `* /` multiplicative tier) and `^` (exponentiation, at its own tighter
/// `BP_POW`). Unary minus still binds tighter than `^` because it is a prefix
/// production (`parse_prefix`) at `BP_UNARY`, not part of this table.
const fn infix_bp(kind: &TokenKind) -> Option<(u8, u8, BinaryOp)> {
    let entry = match kind {
        TokenKind::Keyword(Keyword::Or) => (BP_OR, BinaryOp::Or),
        TokenKind::Keyword(Keyword::And) => (BP_AND, BinaryOp::And),
        TokenKind::Eq => (BP_CMP, BinaryOp::Eq),
        TokenKind::NotEq => (BP_CMP, BinaryOp::Ne),
        TokenKind::Lt => (BP_CMP, BinaryOp::Lt),
        TokenKind::Le => (BP_CMP, BinaryOp::Le),
        TokenKind::Gt => (BP_CMP, BinaryOp::Gt),
        TokenKind::Ge => (BP_CMP, BinaryOp::Ge),
        TokenKind::Concat => (BP_OTHER, BinaryOp::Concat),
        TokenKind::Containment => (BP_OTHER, BinaryOp::Containment),
        TokenKind::ContainedBy => (BP_OTHER, BinaryOp::ContainedBy),
        TokenKind::Arrow => (BP_OTHER, BinaryOp::JsonGet),
        TokenKind::ArrowText => (BP_OTHER, BinaryOp::JsonGetText),
        TokenKind::HashArrow => (BP_OTHER, BinaryOp::JsonPathGet),
        TokenKind::HashArrowText => (BP_OTHER, BinaryOp::JsonPathGetText),
        TokenKind::Question => (BP_OTHER, BinaryOp::JsonExists),
        TokenKind::QuestionPipe => (BP_OTHER, BinaryOp::JsonExistsAny),
        TokenKind::QuestionAmp => (BP_OTHER, BinaryOp::JsonExistsAll),
        TokenKind::Plus => (BP_ADD, BinaryOp::Add),
        TokenKind::Minus => (BP_ADD, BinaryOp::Sub),
        TokenKind::Star => (BP_MUL, BinaryOp::Mul),
        TokenKind::Slash => (BP_MUL, BinaryOp::Div),
        TokenKind::Percent => (BP_MUL, BinaryOp::Mod),
        TokenKind::Caret => (BP_POW, BinaryOp::Pow),
        _ => return None,
    };
    let (bp, op) = entry;
    Some((bp, bp + 1, op))
}

/// The operator a generic [`TokenKind::Op`] spelled `text` names, where pglet implements
/// one.
///
/// 18.6 lexes every one of these as an `Op` — a name resolved against `pg_operator` —
/// and pglet resolves the same names here, at the same binding power (the "any other
/// operator" band). The pattern-matching four are what `LIKE`/`ILIKE` are transformed
/// into there (`~~` and its negations), and `^@` is `starts_with`. A name outside this
/// table and the lexer's own kinds is an operator pglet implements nothing for, and the
/// statement is refused `42883` ([`Parser::owe_unknown_operator`]).
const fn generic_infix_op(text: &str) -> Option<BinaryOp> {
    Some(match text.as_bytes() {
        b"~~" => BinaryOp::Like,
        b"!~~" => BinaryOp::NotLike,
        b"~~*" => BinaryOp::Ilike,
        b"!~~*" => BinaryOp::NotIlike,
        b"^@" => BinaryOp::StartsWith,
        // POSIX regex, narrowed at evaluation to an anchored literal prefix
        // (`executor::expr`'s `eval_regex`); `~*` / `!~*` stay unimplemented.
        b"~" => BinaryOp::RegexMatch,
        b"!~" => BinaryOp::NotRegexMatch,
        _ => return None,
    })
}

/// What the operator loop found at the cursor: the operator pglet implements, or an
/// operator name it does not.
enum Infix {
    Known(BinaryOp),
    /// A name no operator of pglet's answers to — `42883` at the statement's analysis,
    /// as on 18.6, and the statement parses on past a placeholder so an error 18.6
    /// raises earlier still outranks it.
    Unimplemented,
}

impl Parser<'_> {
    /// The infix entry at the cursor: the binding powers and what the operator is.
    ///
    /// The token kinds the lexer gives a spelling of its own go through [`infix_bp`];
    /// a generic [`TokenKind::Op`] is resolved by its **text**, as 18.6 resolves an
    /// `Op` by its name, at the "any other operator" band — which is where `gram.y`
    /// puts `qual_Op` (`%left Op OPERATOR`).
    fn infix_entry(&self) -> Option<(u8, u8, Infix)> {
        if let Some((left_bp, right_bp, op)) = infix_bp(self.ts.peek_kind()) {
            return Some((left_bp, right_bp, Infix::Known(op)));
        }
        if matches!(self.ts.peek_kind(), TokenKind::Op) {
            let entry =
                generic_infix_op(self.ts.peek_text()).map_or(Infix::Unimplemented, Infix::Known);
            return Some((BP_OTHER, BP_OTHER + 1, entry));
        }
        None
    }

    /// Record that the statement names an operator pglet implements nothing for, with
    /// 18.6's own sentence over the operand types it can read.
    ///
    /// The refusal is **deferred** to the statement's end ([`Parser::analysis_refusal`]):
    /// 18.6 raises it in parse analysis, so every error its raw parse raises first — a
    /// grammar error anywhere in the statement — still outranks it (`SELECT 1 ! 1 1` is
    /// `42601` on both engines), and the operands are parsed on past a placeholder so
    /// the rest of the statement is read as 18.6 reads it.
    ///
    /// The operand types are read against the catalog with no FROM relation, so a
    /// literal, a cast and a function call name their type and a **column** reads
    /// `unknown` where 18.6 names the column's type (`EDGES.md` row 184). The verdict
    /// does not depend on them: no operand type gives pglet an operator it does not have.
    fn owe_unknown_operator(&mut self, name: &str, left: Option<&Expr>, right: &Expr) {
        if self.analysis_refusal.is_some() {
            return;
        }
        let scope = crate::planner::PlannerScope::catalog(self.schema, None);
        let ty = |expr: &Expr| {
            // An unadorned string literal is 18.6's `unknown`, which is what it prints
            // for one here — `'a' & 'b'` is `unknown & unknown` there.
            if matches!(expr, Expr::Literal(Literal::String(_))) {
                return "unknown";
            }
            // Read before the coercion pass has run, so through the reader that asks
            // no aggregate for a certainty the pass has not recorded yet.
            crate::types::opt_type_oid_display_name(expr.infer_type_unresolved(&scope))
        };
        let message = left.map_or_else(
            || format!("operator does not exist: {name} {}", ty(right)),
            |left| format!("operator does not exist: {} {name} {}", ty(left), ty(right)),
        );
        self.owe_analysis_refusal(Error::UndefinedFunction(message));
    }

    /// Parse an expression binding at least as tightly as `min_bp`.
    ///
    /// Depth-guarded (nesting only — the operator loop is iterative).
    pub(super) fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr> {
        self.enter()?;
        let result = self.parse_expr_bp_inner(min_bp);
        self.leave();
        result
    }

    /// Continue the operator loop with `left` **already parsed**, returning the
    /// resulting expression and whether the loop consumed anything.
    ///
    /// The entry point for a caller that classified an expression's leading tokens
    /// itself and needs to know whether more of an expression follows —
    /// `parse_column_default`, which recognizes the closed `ColumnDefault` forms at
    /// the token level and must tell `DEFAULT 5` from `DEFAULT 5 + a`. Returning the
    /// flag rather than making the caller re-derive "does the next token continue an
    /// expression at `min_bp`" keeps that question answered in exactly one place:
    /// a caller-side predicate would be a second copy of the loop's own guards, free
    /// to drift from them.
    pub(super) fn continue_expr_bp(&mut self, left: Expr, min_bp: u8) -> Result<(Expr, bool)> {
        self.enter()?;
        let result = self.continue_expr_bp_inner(left, min_bp);
        self.leave();
        result
    }

    fn parse_expr_bp_inner(&mut self, min_bp: u8) -> Result<Expr> {
        let left = self.parse_prefix()?;
        self.continue_expr_bp_inner(left, min_bp)
            .map(|(expr, _)| expr)
    }

    fn continue_expr_bp_inner(&mut self, left: Expr, min_bp: u8) -> Result<(Expr, bool)> {
        // Whether the loop consumed anything is **derived from the token position**,
        // not set per arm. Every `continue` arm would otherwise have to remember to
        // raise a flag, and a future arm that forgot would silently make
        // `DEFAULT 5 + a` read as a bare `DEFAULT 5` — a wrong stored default with no
        // error anywhere. Every arm advances the stream by at least one token and
        // token start offsets strictly increase, so "the position moved" and "the
        // loop consumed something" are the same fact, derived once.
        let start = self.ts.pos();
        let mut chain = OperatorChain::new(left, self.depth);
        let looped = self.operator_loop(&mut chain, min_bp);
        // A refusal the chain recorded stands even when the loop then failed: the
        // statement's end settles that error against it.
        if chain.refused {
            self.owe_depth_refusal();
        }
        looped?;
        let (expr, refused) = chain.finish();
        if refused {
            self.owe_depth_refusal();
        }
        Ok((expr, self.ts.pos() != start))
    }

    /// The operator loop of [`Self::continue_expr_bp_inner`]: extend `chain` with every
    /// postfix and infix operator binding at least as tightly as `min_bp`.
    fn operator_loop(&mut self, chain: &mut OperatorChain, min_bp: u8) -> Result<()> {
        loop {
            // `::` cast — tightest, postfix.
            if BP_CAST >= min_bp && self.ts.at(&TokenKind::Cast) {
                self.ts.bump()?;
                chain.wrap(|left| self.parse_cast_target(left))?;
                continue;
            }
            // `AT TIME ZONE zone` — high-binding postfix. Excluded from `b_expr`
            // (`gram.y` has no `AT` production there), so the mode stops the loop
            // and the caller reports the syntax error 18.6 reports.
            if !self.b_expr && BP_AT >= min_bp && self.ts.at_kw(Keyword::At) {
                chain.wrap(|left| self.parse_at_time_zone(left))?;
                continue;
            }
            // `IS [NOT] NULL` / `IS [NOT] DISTINCT FROM …` — postfix. `b_expr` keeps
            // the `DISTINCT FROM` half and drops the `IS NULL` family, so the arm is
            // entered either way and `parse_is_postfix` refuses the excluded half
            // once it has read far enough to tell them apart (the cursor's lookahead
            // is two tokens and `IS NOT DISTINCT` needs three).
            if BP_IS >= min_bp && self.ts.at_kw(Keyword::Is) {
                chain.wrap(|left| self.parse_is_postfix(left))?;
                continue;
            }
            // `x ISNULL` / `x NOTNULL` — `gram.y`'s `a_expr ISNULL` / `a_expr NOTNULL`,
            // the one-word spellings of `IS [NOT] NULL`, at `IS`'s precedence
            // (`%nonassoc IS ISNULL NOTNULL`). `a_expr` only: `b_expr` has neither
            // production, so the mode stops the loop and the word is the caller's
            // syntax error, as it is 18.6's.
            if !self.b_expr && BP_IS >= min_bp {
                let negated = match self.ts.peek_kind() {
                    TokenKind::Keyword(Keyword::Isnull) => Some(false),
                    TokenKind::Keyword(Keyword::Notnull) => Some(true),
                    _ => None,
                };
                if let Some(negated) = negated {
                    self.ts.bump()?;
                    chain.wrap(|left| {
                        Ok(if negated {
                            Expr::IsNotNull(left)
                        } else {
                            Expr::IsNull(left)
                        })
                    })?;
                    continue;
                }
            }
            // `[NOT] IN (...)` / `BETWEEN …` / `LIKE`/`ILIKE …` — predicate band,
            // excluded from `b_expr` entirely.
            if !self.b_expr
                && BP_PRED >= min_bp
                && let Some(negated) = self.predicate_negation()
            {
                chain.wrap(|left| self.parse_predicate(left, negated))?;
                continue;
            }
            // Pure binary-operator table — with a quantified `ANY`/`ALL`/`SOME`
            // array on the right of a comparison operator.
            if let Some((left_bp, right_bp, entry)) = self.infix_entry() {
                if left_bp < min_bp {
                    break;
                }
                let op = match entry {
                    Infix::Known(op) => op,
                    // An operator pglet has none of: the statement is refused at its
                    // end, and its operands are parsed as 18.6 parses them.
                    Infix::Unimplemented => {
                        let name = self.ts.peek_text().to_string();
                        self.ts.bump()?;
                        let right = self.parse_expr_bp(right_bp)?;
                        let left = chain.take_operand();
                        self.owe_unknown_operator(&name, Some(&left), &right);
                        chain.refuse_operand();
                        continue;
                    }
                };
                // `AND`/`OR` are `a_expr`-only; the rest of the table is in `b_expr`.
                if self.b_expr && matches!(op, BinaryOp::And | BinaryOp::Or) {
                    break;
                }
                self.ts.bump()?;
                if is_comparison(op) && self.at_quantifier() {
                    // The quantified comparison is an `a_expr` production only
                    // (`a_expr subquery_Op sub_type …`); `b_expr` has no `sub_type`
                    // arm at all, so 18.6 answers `syntax error at or near "ANY"` —
                    // raised at the quantifier keyword, before its operand is looked
                    // at, exactly as the `NOT` arm above is. Measured on 18.6 for
                    // `ANY` / `ALL` / `SOME`, in both `b_expr` spellings
                    // (`CREATE TABLE`'s column constraint and `ALTER TABLE … ADD
                    // COLUMN`'s); the `SET DEFAULT` spelling is full `a_expr` and is
                    // accepted there and here, which is the control.
                    if self.b_expr {
                        return Err(self.ts.syntax_error_at_or_near());
                    }
                    let (all, right, sub_select) = self.parse_quantified()?;
                    chain.wrap(|left| {
                        Ok(Expr::AnyAll {
                            left,
                            op,
                            right: Box::new(right),
                            all,
                            sub_select,
                        })
                    })?;
                } else {
                    let right = self.parse_expr_bp(right_bp)?;
                    if matches!(op, BinaryOp::And | BinaryOp::Or) {
                        chain.push_bool(op, right);
                    } else {
                        chain.wrap(|left| {
                            Ok(Expr::BinaryOp {
                                left,
                                op,
                                right: Box::new(right),
                            })
                        })?;
                    }
                }
                // Comparison operators are non-associative (Postgres `%nonassoc`):
                // once one is consumed at this level, a directly-following
                // comparison — of any of the six, `1 = 1 = true` / `1 < 2 = true` —
                // is a syntax error, not a left-associative re-bind. (The right
                // operand parsed at `BP_CMP + 1`, so it never absorbed the trailing
                // comparison; it is still current here.)
                if is_comparison(op)
                    && matches!(infix_bp(self.ts.peek_kind()), Some((.., next)) if is_comparison(next))
                {
                    return Err(self.ts.error("operator is not associative"));
                }
                continue;
            }
            break;
        }
        Ok(())
    }

    /// Parse a prefix expression: a unary operator then its operand, or a primary.
    fn parse_prefix(&mut self) -> Result<Expr> {
        // `NOT` is an `a_expr` production only, so under `b_expr` it is a syntax
        // error at the `NOT` itself — 18.6's `syntax error at or near "NOT"`,
        // raised *before* the operand is looked at (`DEFAULT NOT (true)` and
        // `DEFAULT NOT EXISTS (SELECT 1)` are both that, not their operand's
        // verdict).
        if self.b_expr && self.ts.at_kw(Keyword::Not) {
            return Err(self.ts.error("syntax error at or near \"NOT\""));
        }
        if self.ts.at(&TokenKind::Minus) {
            self.ts.bump()?;
            return Ok(negate(self.parse_expr_bp(BP_UNARY)?));
        }
        if self.ts.at(&TokenKind::Op) {
            // `gram.y`'s `qual_Op a_expr %prec Op` — a prefix operator, whose operand
            // takes everything binding tighter than the operator band. pglet implements
            // no prefix operator, so every one is `42883` at the statement's analysis.
            let name = self.ts.peek_text().to_string();
            self.ts.bump()?;
            let operand = self.parse_expr_bp(BP_OTHER + 1)?;
            self.owe_unknown_operator(&name, None, &operand);
            return Ok(Expr::Literal(Literal::Null));
        }
        if self.ts.at(&TokenKind::Plus) {
            // Unary plus is value-preserving but is **kept as a node**, not folded
            // away: Postgres's grammar leaves `'+' a_expr` an operator node, where
            // unary minus over a numeric constant folds into the constant (`negate`,
            // `doNegate`) — and that asymmetry is what stops `GROUP BY +1` /
            // `ORDER BY +1` from reading as an ordinal while `-1` does.
            self.ts.bump()?;
            return Ok(Expr::UnaryPlus(Box::new(self.parse_expr_bp(BP_UNARY)?)));
        }
        if self.ts.at_kw(Keyword::Not) {
            // `NOT EXISTS (…)` folds into `Exists { negated: true }` rather than
            // `Not(Exists { negated: false })`.
            if matches!(self.ts.peek2_kind(), TokenKind::Keyword(Keyword::Exists)) {
                self.ts.bump()?; // NOT
                return self.parse_exists(true);
            }
            self.ts.bump()?;
            return Ok(Expr::Not(Box::new(self.parse_expr_bp(BP_NOT)?)));
        }
        self.parse_primary()
    }

    /// Parse a `CASE … WHEN … THEN … [ELSE …] END` expression, both the searched
    /// form (`CASE WHEN cond THEN …`) and the simple form (`CASE operand WHEN
    /// value THEN …`).
    fn parse_case(&mut self) -> Result<Expr> {
        self.ts.expect_kw(Keyword::Case)?;
        // The simple form has an operand before the first `WHEN`.
        let operand = if self.ts.at_kw(Keyword::When) {
            None
        } else {
            Some(Box::new(self.parse_expr_bp(0)?))
        };
        let mut when_clauses = Vec::new();
        while self.ts.eat_kw(Keyword::When)? {
            let condition = self.parse_expr_bp(0)?;
            self.ts.expect_kw(Keyword::Then)?;
            let result = self.parse_expr_bp(0)?;
            when_clauses.push((condition, result));
        }
        if when_clauses.is_empty() {
            return Err(self.ts.error("CASE requires at least one WHEN branch"));
        }
        let else_result = if self.ts.eat_kw(Keyword::Else)? {
            Some(Box::new(self.parse_expr_bp(0)?))
        } else {
            None
        };
        self.ts.expect_kw(Keyword::End)?;
        Ok(Expr::Case {
            operand,
            when_clauses,
            else_result,
        })
    }

    /// Parse `INTERVAL 'literal' [<field qualifier>]` into a [`Value::Interval`],
    /// carried as a constant `Expr::Bound` — `gram.y`'s
    /// `ConstInterval Sconst opt_interval`.
    ///
    /// The literal's own grammar (verbose / SQL-standard / ISO-8601) is
    /// [`crate::types::PgInterval::parse`]'s; a malformed one fails loud (`22007`).
    /// The **qualifier** does two things, in this order, and both were missing:
    /// leaving it unparsed made `INTERVAL '90' MINUTE` read as `INTERVAL '90'`
    /// (ninety *seconds*) under an output alias `minute`, answering `00:01:30`
    /// where 18.6 answers `01:30:00` — a silent wrong, not a parse wall.
    ///
    /// 1. A **bare number** literal takes the qualifier's unit
    ///    (`interval_from_unit`), which is what `DecodeInterval` does with its
    ///    `range` argument: `'1' DAY` is one day, `'90' MINUTE` ninety minutes.
    /// 2. The result is then **masked** to the qualifier: fields to the right of
    ///    the named one are zeroed, those to its left kept
    ///    (`timestamp.c`'s `AdjustIntervalForTypmod`) — so `'1 day 3 hours' DAY` is
    ///    `1 day`, `'1 year 3 months' YEAR` is `1 year`, and `'1 day' MINUTE` is
    ///    still `1 day` (a MINUTE mask truncates only the sub-day remainder).
    ///
    /// Only the **single-field** qualifiers are honored. The `X TO Y` ranges fail
    /// loud (`0A000`) rather than being masked-only: a range also re-reads the
    /// literal's *unqualified* fields (`'3:04' MINUTE TO SECOND` is `00:03:04` on
    /// 18.6 where this parser's colon grammar reads `03:04:00`), so masking without
    /// that re-read would answer a wrong value silently — the defect this whole
    /// arm exists to close.
    fn parse_interval_literal(&mut self) -> Result<Expr> {
        self.ts.bump()?; // `interval`
        let tok = self.ts.advance()?; // the string literal
        let body = self.ts.text(&tok).to_string();
        let Some(unit) = self.parse_interval_qualifier()? else {
            let iv = PgInterval::parse(&body)?;
            return Ok(Expr::Bound(Value::Interval(iv)));
        };
        let iv = if is_bare_interval_number(&body) {
            interval_from_unit(&body, unit)?
        } else {
            PgInterval::parse(&body)?
        };
        Ok(Expr::Bound(Value::Interval(mask_interval(iv, unit))))
    }

    /// Read `opt_interval`'s single-field form, returning the field it names.
    ///
    /// The six field words are contextual identifiers (unquoted only), so a column
    /// named `day` is unaffected; `WEEK` is deliberately absent because it is absent
    /// from `opt_interval` too — `INTERVAL '1' WEEK` is `00:00:01` under an alias
    /// `week` on 18.6. A `TO` after the field word is the range form: loud
    /// [`Error::NotImplemented`] (see [`Self::parse_interval_literal`]), consuming
    /// nothing further, since a wrong value would otherwise be silent.
    fn parse_interval_qualifier(&mut self) -> Result<Option<IntervalField>> {
        let field = if self.eat_ident("year")? {
            IntervalField::Year
        } else if self.eat_ident("month")? {
            IntervalField::Month
        } else if self.eat_ident("day")? {
            IntervalField::Day
        } else if self.eat_ident("hour")? {
            IntervalField::Hour
        } else if self.eat_ident("minute")? {
            IntervalField::Minute
        } else if self.eat_ident("second")? {
            IntervalField::Second
        } else {
            return Ok(None);
        };
        if self.ts.at_kw(Keyword::To) {
            return Err(Error::NotImplemented(
                "INTERVAL field ranges (`<field> TO <field>`) are not supported".into(),
            ));
        }
        Ok(Some(field))
    }

    /// Try the **typed-constant** productions of `gram.y`'s `AexprConst` —
    /// `ConstTypename Sconst` and `func_name [ '(' typmod ')' ] Sconst` — returning
    /// `None` (cursor untouched) when the cursor is not on one.
    ///
    /// A typed constant is the string literal read as that type, so it builds the same
    /// cast IR `'…'::type` builds ([`Self::build_cast`]): same node kinds, same value,
    /// same declared type OID, same input-conversion SQLSTATE (`22007` for
    /// `DATE 'not-a-date'`, `22P02` for `INT4 'x'`), the same enforcement of a
    /// **spelled** modifier (`NUMERIC(5,2) '1.5'` → `1.50`, `VARCHAR(3) 'abcd'` →
    /// `abc`), the same output column name (the type's canonical spelling, measured on
    /// 18.6), and the same deparse — `pg_get_constraintdef` renders a stored
    /// `CHECK (d > DATE '…')` as `(d > '…'::date)` there, which is exactly what
    /// `deparse::render_expr` already emits for a cast.
    ///
    /// **What the two positions do not share is the fixed-length default**, which is
    /// why the type name resolves through
    /// [`crate::executor::resolve_constant_type`] here and
    /// [`crate::executor::resolve_column_type`] at the cast: `gram.y`'s
    /// `ConstCharacter` exists to keep `CHAR 'hi'` from taking the `char(1)` an
    /// `'hi'::char` takes. So `char 'ab'` is `ab` while `'ab'::char` is `a`.
    ///
    /// **The default is the parenless spelling's**, so a modifier that *is* spelled is
    /// the same at both — `char(1) 'ab'` is `a` again, and a length the resolver
    /// **refuses** (`char(0) 'ab'`) answers what `'ab'::char(0)` answers, the `22023`.
    /// The `varchar` family spells no default at either position and cannot see the
    /// split at all.
    ///
    /// **Two dispatch shapes, because only one of them is decidable from the
    /// cursor.** An identifier immediately followed by a string literal is a typed
    /// constant *unconditionally* — 18.6 answers `SELECT foo 'x'` with
    /// `42704 type "foo" does not exist` even when `foo` is a real column, so that
    /// case commits with no lookahead beyond the window. A **multi-word or
    /// modifier-bearing** type name (`DOUBLE PRECISION '…'`, `CHARACTER VARYING '…'`,
    /// `TIMESTAMP WITH TIME ZONE '…'`, `NUMERIC(5,2) '…'`) is not decidable at two
    /// tokens, and every one of its lead words is a legal bare column name on both
    /// engines (`SELECT double FROM tl` resolves on 18.6 and here), so it parses
    /// speculatively from a [`checkpoint`](super::stream::TokenStream::checkpoint)
    /// and puts the cursor back when no string literal follows the type name.
    ///
    /// **The speculation has a third outcome: a window it commits to.** Where the lead
    /// word and the `(` after it can begin nothing but a type name, a failure inside
    /// the speculation is the statement's answer and the cursor is not put back — a
    /// restore would demote that verdict to whatever the function-call path says next.
    /// [`commits_on_open_paren`] is the membership test and states why `float` is its
    /// member and why the bare word is not.
    ///
    /// The type name is the **scalar** production only: `int4[] '{1,2}'` is `42601`
    /// on 18.6 (an array typed constant is spelled `_int4 '{1,2}'`), and refusing the
    /// `[]` suffix here leaves `int4[] '…'` on the column path, where it stays a
    /// syntax error.
    fn try_typed_constant(&mut self) -> Result<Option<Expr>> {
        if !matches!(self.ts.peek_kind(), TokenKind::Identifier) {
            return Ok(None);
        }
        if matches!(self.ts.peek2_kind(), TokenKind::String) {
            return self.parse_typed_constant().map(Some);
        }
        if !is_multiword_type_lead(self.ts.peek_text()) {
            return Ok(None);
        }
        let committed = commits_on_open_paren(self.ts.peek_text())
            && matches!(self.ts.peek2_kind(), TokenKind::LParen);
        let checkpoint = self.ts.checkpoint();
        // **Only a *shape* failure restores**, and the two phases are separated here
        // to keep them apart. Reading a scalar type name and then a string literal is
        // the question "are these tokens a typed constant"; *resolving* that type is
        // the answer to a statement 18.6 also reads as one. Running both under a
        // single `if let Ok(…)` swallowed the second as though it were the first, and
        // demoted a documented verdict to a syntax error:
        // `SELECT time with time zone '10:00'` is the `TIMETZ` rule's `42704` through
        // the single-word `timetz` spelling and was `42601` through this one.
        let shape = self.parse_scalar_type_name().and_then(|canonical| {
            self.ts
                .expect(&TokenKind::String)
                .map(|tok| (canonical, tok))
        });
        let (canonical, tok) = match shape {
            Ok(read) => read,
            // A **committed** window has no decline left: whatever went wrong is the
            // statement's answer, as it is on 18.6 (see `commits_on_open_paren`).
            Err(e) if committed => return Err(e),
            Err(_) => {
                self.ts.restore(checkpoint);
                return Ok(None);
            }
        };
        self.typed_constant_from(&canonical, &tok).map(Some)
    }

    /// Parse one typed constant: a scalar type name, then the string literal it
    /// types. Assumes [`Self::try_typed_constant`] has decided this is one.
    fn parse_typed_constant(&mut self) -> Result<Expr> {
        let canonical = self.parse_scalar_type_name()?;
        let tok = self.ts.expect(&TokenKind::String)?;
        self.typed_constant_from(&canonical, &tok)
    }

    /// Resolve a read type name against the catalog and build the constant — the
    /// second phase of [`Self::try_typed_constant`], whose errors are **verdicts**
    /// (`42704` for a type that does not resolve) rather than a decline.
    ///
    /// The resolver is [`crate::executor::resolve_constant_type`], which is
    /// [`crate::executor::resolve_column_type`] minus the parenless-`char`
    /// fixed-length default — the part of `gram.y`'s `ConstTypename` pglet carries.
    fn typed_constant_from(&self, canonical: &str, literal: &Token) -> Result<Expr> {
        let text = self.ts.text(literal).to_string();
        let resolved = crate::executor::resolve_constant_type(canonical, self.schema)?;
        Ok(self.build_resolved_cast(Box::new(Expr::Literal(Literal::String(text))), &resolved))
    }

    /// Parse `EXISTS (subquery)`; `negated` is set by a preceding `NOT`.
    fn parse_exists(&mut self, negated: bool) -> Result<Expr> {
        self.ts.expect_kw(Keyword::Exists)?;
        self.ts.expect(&TokenKind::LParen)?;
        let plan = self.in_expr_subquery(Self::parse_select)?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(Expr::Exists {
            plan: Box::new(plan),
            negated,
        })
    }

    /// Parse the `SUBSTRING(...)` special form (and its plain-call comma form).
    ///
    /// `SUBSTRING(x FROM n FOR m)` / `(x FROM n)` / `(x FOR m)` (FOR-only defaults the
    /// start to 1) build the same call the comma form `SUBSTRING(x, n, m)` does — both
    /// converge on one `StringFunc::Substring` node via `build_call`, so arity is
    /// checked in one place. The `FROM`/`FOR` clauses may appear in either order.
    fn parse_substring(&mut self) -> Result<Expr> {
        self.ts.bump()?; // `substring`
        self.ts.expect(&TokenKind::LParen)?;
        let first = self.parse_expr_bp(0)?;
        if self.ts.at_kw(Keyword::From) || self.ts.at_kw(Keyword::For) {
            let mut start = None;
            let mut length = None;
            loop {
                if self.ts.eat_kw(Keyword::From)? {
                    start = Some(self.parse_expr_bp(0)?);
                } else if self.ts.eat_kw(Keyword::For)? {
                    length = Some(self.parse_expr_bp(0)?);
                } else {
                    break;
                }
            }
            self.ts.expect(&TokenKind::RParen)?;
            // FOR-only → the start defaults to 1 (Postgres).
            let start = start.unwrap_or(Expr::Literal(Literal::Int(1)));
            let args = match length {
                Some(len) => vec![first, start, len],
                None => vec![first, start],
            };
            return crate::planner::build_call("substring", args, SetQuantifier::None, false);
        }
        // Plain comma form: `SUBSTRING(x, n[, m])`.
        let mut args = vec![first];
        while self.ts.eat(&TokenKind::Comma)? {
            args.push(self.parse_expr_bp(0)?);
        }
        self.ts.expect(&TokenKind::RParen)?;
        crate::planner::build_call("substring", args, SetQuantifier::None, false)
    }

    /// Parse the `TRIM(...)` special form.
    ///
    /// `TRIM([BOTH|LEADING|TRAILING] [chars] FROM x)` maps to `btrim`/`ltrim`/`rtrim`
    /// (BOTH or omitted → btrim); `TRIM(chars FROM x)` → `btrim(x, chars)` (string
    /// first); `TRIM(x)` / `TRIM(FROM x)` → the 1-arg space default. BOTH/LEADING/
    /// TRAILING are contextual identifiers (via `eat_ident`), so they stay usable as
    /// column names rather than becoming reserved words.
    fn parse_trim(&mut self) -> Result<Expr> {
        self.ts.bump()?; // `trim`
        self.ts.expect(&TokenKind::LParen)?;
        let name = if self.eat_ident("leading")? {
            "ltrim"
        } else if self.eat_ident("trailing")? {
            "rtrim"
        } else {
            let _ = self.eat_ident("both")?; // BOTH (or none) → the default btrim
            "btrim"
        };
        let args = if self.ts.eat_kw(Keyword::From)? {
            // `TRIM([dir] FROM x)` — no character set, default space.
            vec![self.parse_expr_bp(0)?]
        } else {
            let first = self.parse_expr_bp(0)?;
            if self.ts.eat_kw(Keyword::From)? {
                // `TRIM([dir] chars FROM x)` → trim(x, chars) — string first.
                let string = self.parse_expr_bp(0)?;
                vec![string, first]
            } else {
                // `TRIM([dir] x)` — just the string, default space.
                vec![first]
            }
        };
        self.ts.expect(&TokenKind::RParen)?;
        crate::planner::build_call(name, args, SetQuantifier::None, false)
    }

    /// Parse the `POSITION(sub IN str)` special form → `StringFunc::Position`, which
    /// evaluates identically to `strpos(str, sub)` (the operands are swapped so the
    /// haystack comes first) but keeps Postgres's `position` output column name. The
    /// operand before `IN` parses above the predicate band so `IN` terminates it
    /// rather than being read as an `IN`-list.
    fn parse_position(&mut self) -> Result<Expr> {
        self.ts.bump()?; // `position`
        self.ts.expect(&TokenKind::LParen)?;
        let needle = self.parse_expr_bp(BP_PRED + 1)?;
        self.ts.expect_kw(Keyword::In)?;
        let haystack = self.parse_expr_bp(0)?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(Expr::StringFn {
            func: StringFunc::Position,
            args: vec![haystack, needle],
        })
    }

    /// Parse the `EXTRACT(...)` forms → `Expr::Extract { retnumeric: true }`.
    ///
    /// `EXTRACT(field FROM source)` is the SQL special form: `field` is a bare
    /// identifier / keyword (`year`, `dow`, `epoch`, …) or a string constant, folded
    /// to a text literal resolved at eval time. The plain-call comma form
    /// `extract(field, source)` (where `field` is any text expression) is also
    /// accepted — both build the same node, differing only in how the field is
    /// written. The FROM form is detected by a `FROM` keyword right after the field
    /// word; otherwise the comma form is parsed. `date_part` (the float8 twin) has no
    /// special grammar — it routes through `build_call`.
    fn parse_extract(&mut self) -> Result<Expr> {
        self.ts.bump()?; // `extract`
        self.ts.expect(&TokenKind::LParen)?;
        // `EXTRACT(field FROM source)`: a bare field word (identifier / keyword /
        // string constant) immediately followed by `FROM`.
        let is_field_word = matches!(
            self.ts.peek_kind(),
            TokenKind::Identifier
                | TokenKind::QuotedIdentifier
                | TokenKind::String
                | TokenKind::Keyword(_)
        );
        if is_field_word && matches!(self.ts.peek2_kind(), TokenKind::Keyword(Keyword::From)) {
            let tok = self.ts.advance()?; // the field word
            // A bare field word is an identifier, so it downcases; a quoted one and a
            // string constant stay verbatim. Either way `ExtractField::from_name`
            // resolves case-insensitively, because the field reaches `timestamp_part`
            // as a *value* that `DecodeUnits` downcases — `EXTRACT("YEAR" FROM …)`
            // resolves on 18.6. Folding here only decides how an unrecognized unit is
            // spelled back in the `22023` message (`unit "foo" not recognized`).
            let field_name = self.ts.name(&tok);
            self.ts.expect_kw(Keyword::From)?;
            let source = self.parse_expr_bp(0)?;
            self.ts.expect(&TokenKind::RParen)?;
            return Ok(Expr::Extract {
                source: Box::new(source),
                field: Box::new(Expr::Literal(Literal::String(field_name))),
                retnumeric: true,
            });
        }
        // Plain comma form: `extract(field_expr, source)`.
        let field = self.parse_expr_bp(0)?;
        self.ts.expect(&TokenKind::Comma)?;
        let source = self.parse_expr_bp(0)?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(Expr::Extract {
            source: Box::new(source),
            field: Box::new(field),
            retnumeric: true,
        })
    }

    /// Parse the `IS …` postfix family applied to `left`.
    fn parse_is_postfix(&mut self, left: Box<Expr>) -> Result<Expr> {
        self.ts.expect_kw(Keyword::Is)?;
        let negated = self.ts.eat_kw(Keyword::Not)?;
        // `b_expr` admits `IS [NOT] DISTINCT FROM` and `IS [NOT] DOCUMENT` and
        // nothing else in this family, so every other spelling is 18.6's
        // `42601` here (measured: `DEFAULT 5 IS NULL` and
        // `DEFAULT (5 IS NULL) IS NULL` both `syntax error at or near "NULL"`).
        // The test sits after the `IS`/`NOT` are consumed because telling
        // `IS NOT DISTINCT` from `IS NOT NULL` needs a third token of lookahead
        // and the cursor carries two.
        if self.b_expr && !self.ts.at_kw(Keyword::Distinct) {
            return Err(self.ts.expected("DISTINCT"));
        }
        if self.ts.eat_kw(Keyword::Null)? {
            return Ok(if negated {
                Expr::IsNotNull(left)
            } else {
                Expr::IsNull(left)
            });
        }
        if self.ts.eat_kw(Keyword::Distinct)? {
            self.ts.expect_kw(Keyword::From)?;
            let right = self.parse_expr_bp(BP_IS + 1)?;
            let op = if negated {
                BinaryOp::IsNotDistinctFrom
            } else {
                BinaryOp::IsDistinctFrom
            };
            return Ok(Expr::BinaryOp {
                left,
                op,
                right: Box::new(right),
            });
        }
        // `IS [NOT] TRUE / FALSE / UNKNOWN` — `gram.y`'s `BooleanTest`. `TRUE` and
        // `FALSE` are reserved keywords; `UNKNOWN` is not in the keyword table at all
        // (it is unreserved in Postgres too), so it reads as a contextual identifier
        // and stays usable as a column name.
        let test = if self.ts.eat_kw(Keyword::True)? {
            if negated {
                BoolTest::NotTrue
            } else {
                BoolTest::True
            }
        } else if self.ts.eat_kw(Keyword::False)? {
            if negated {
                BoolTest::NotFalse
            } else {
                BoolTest::False
            }
        } else if self.eat_ident("unknown")? {
            if negated {
                BoolTest::NotUnknown
            } else {
                BoolTest::Unknown
            }
        } else {
            return Err(self.ts.error("unsupported IS predicate"));
        };
        Ok(Expr::BooleanTest { expr: left, test })
    }

    /// Whether a predicate-band keyword (`IN`/`BETWEEN`/`LIKE`/`ILIKE`), possibly
    /// preceded by `NOT`, follows; `Some(negated)` if so. Does not consume.
    fn predicate_negation(&self) -> Option<bool> {
        if is_pred_keyword(self.ts.peek_kind()) {
            Some(false)
        } else if self.ts.at_kw(Keyword::Not) && is_pred_keyword(self.ts.peek2_kind()) {
            Some(true)
        } else {
            None
        }
    }

    /// Dispatch a predicate-band postfix (`[NOT] IN/BETWEEN/LIKE/ILIKE`).
    fn parse_predicate(&mut self, left: Box<Expr>, negated: bool) -> Result<Expr> {
        if negated {
            self.ts.expect_kw(Keyword::Not)?;
        }
        if self.ts.at_kw(Keyword::In) {
            self.parse_in_list(left, negated)
        } else if self.ts.at_kw(Keyword::Between) {
            self.parse_between(*left, negated)
        } else {
            self.parse_like(left, negated)
        }
    }

    /// Parse `[NOT] IN (v1, v2, …)` or `[NOT] IN (SELECT …)`.
    ///
    /// Which of the two it is follows 18.6's `in_expr: select_with_parens | '(' expr_list
    /// ')'`, decided by [`Self::at_select_with_parens`] rather than by the token after the
    /// `(`: `x IN ((SELECT a FROM e))` and `x IN ((SELECT a FROM e) UNION SELECT 1)` are
    /// sub-selects there, at any depth of parentheses, while `x IN ((SELECT a FROM e), 1)`
    /// and `x IN ((SELECT a FROM e) + 0)` are lists holding a scalar subquery. Read as a
    /// list, the first spelling compared `x` with the scalar subquery's `NULL` over an
    /// empty `e`, so `NOT IN` was `NULL` for every row where 18.6 answers `true`.
    fn parse_in_list(&mut self, left: Box<Expr>, negated: bool) -> Result<Expr> {
        self.ts.expect_kw(Keyword::In)?;
        let sub_select = self.at_select_with_parens();
        self.ts.expect(&TokenKind::LParen)?;
        if sub_select {
            // `parse_select` reads a leading `(` as a parenthesized query, so the
            // doubled spelling and a set operation over one arrive here alike.
            let plan = self.in_expr_subquery(Self::parse_select)?;
            self.ts.expect(&TokenKind::RParen)?;
            return Ok(Expr::InSubquery {
                expr: left,
                plan: Box::new(plan),
                negated,
            });
        }
        if self.ts.at_kw(Keyword::With) {
            return Err(Error::NotImplemented(
                "IN (WITH …) not yet supported in the native parser".into(),
            ));
        }
        let list = self.comma_list_until(&TokenKind::RParen, |p| p.parse_expr_bp(0))?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(Expr::InList {
            expr: left,
            list,
            negated,
        })
    }

    /// Whether the `(` at the cursor opens what 18.6's grammar reads as a
    /// `select_with_parens` where an expression list could also begin — the body of an
    /// `IN (…)` or an `ANY (…)`. Does not consume.
    ///
    /// 18.6's rule, from `gram.y`: a parenthesized group is a query when it starts with
    /// `SELECT`, or when it starts with a parenthesized query and that inner query is
    /// followed by the group's `)` (`select_with_parens: '(' select_with_parens ')'`, the
    /// reading bison prefers over a scalar subquery in an expression list), a set
    /// operator or a clause over a whole query (`select_no_parens`). Anything else after
    /// the inner query — `,`, an operator, a cast — makes the group an expression. A
    /// `VALUES`, `WITH` or `TABLE` body is not recognised here and keeps the refusal it
    /// has in expression position.
    ///
    /// A token scan from a checkpoint the cursor is put back to: nothing is parsed, so no
    /// parse-time state moves. A lexical error ends the scan with `false`, and the parse
    /// that follows meets the same error.
    fn at_select_with_parens(&mut self) -> bool {
        if !self.ts.at(&TokenKind::LParen) {
            return false;
        }
        let start = self.ts.checkpoint();
        let verdict = self.scan_select_with_parens().unwrap_or(false);
        self.ts.restore(start);
        verdict
    }

    /// [`Self::at_select_with_parens`]'s scan over the group at the cursor, a `(`.
    ///
    /// It does not recurse, so it takes no stack a level: the run of leading `(` is
    /// counted, the innermost group read for its `SELECT`, and each enclosing group,
    /// innermost first, judged by what follows the group inside it. A run deeper than the
    /// parser's nesting admits ([`Self::admits_nesting`]) is answered `false` before it
    /// is read further, and the group is then parsed as an expression, which refuses the
    /// nesting where [`Self::enter`] refuses any nesting that deep.
    fn scan_select_with_parens(&mut self) -> Result<bool> {
        let mut groups = 0usize;
        while self.ts.at(&TokenKind::LParen) {
            groups += 1;
            if !self.admits_nesting(groups) {
                return Ok(false);
            }
            self.ts.bump()?;
        }
        if !self.ts.at_kw(Keyword::Select) {
            return Ok(false);
        }
        for group in 0..groups {
            let inner_is_query = group == 0
                || self.ts.at(&TokenKind::RParen)
                || self.at_set_op()
                || self.at_query_tail();
            if !inner_is_query {
                return Ok(false);
            }
            self.skip_to_group_end()?;
        }
        Ok(true)
    }

    /// Consume the rest of the group the cursor is inside, through its matching `)` — or
    /// to the end of the text, which the parse that follows refuses.
    fn skip_to_group_end(&mut self) -> Result<()> {
        let mut depth = 1usize;
        loop {
            match self.ts.peek_kind() {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth -= 1,
                TokenKind::Eof => return Ok(()),
                _ => {}
            }
            self.ts.bump()?;
            if depth == 0 {
                return Ok(());
            }
        }
    }

    /// Whether the parser's nesting bound admits `levels` more levels than it is at:
    /// the two limits [`Self::enter`] refuses past, [`MAX_DEPTH`](super::MAX_DEPTH) and
    /// the thread's tree-depth limit ([`crate::depth::check`]), without the byte check,
    /// which a scan that does not recurse has no frames for.
    fn admits_nesting(&self, levels: usize) -> bool {
        let depth = self.depth + levels;
        depth <= super::MAX_DEPTH && crate::depth::check(depth).is_ok()
    }

    /// Parse `[NOT] BETWEEN low AND high`, desugaring to comparisons so the
    /// desugared `AND`-of-comparisons stays range-detectable.
    ///
    /// The two bounds are read as `gram.y`'s production reads them, `a_expr BETWEEN
    /// b_expr AND a_expr %prec BETWEEN`:
    ///
    /// - `low` is a **`b_expr`** — no `AND`/`OR`/`NOT`, no `IS [NOT] NULL` / `ISNULL` /
    ///   `NOTNULL`, no predicate band — so `BETWEEN`'s own `AND` ends it and `true BETWEEN
    ///   NULL IS NULL AND true` is `42601`, as on 18.6;
    /// - `high` takes only what binds tighter than `BETWEEN` (`BP_PRED + 1`), so an
    ///   operator binding below it — a comparison, `IS …`, `ISNULL` — takes the whole
    ///   `BETWEEN` as its left operand: `NULL::bool BETWEEN false AND true IS NULL` is `t`;
    /// - `BETWEEN` is `%nonassoc` with `IN`, `LIKE`, `ILIKE` and their `NOT` spellings, so
    ///   one of those directly after the bound is the `42601` 18.6 answers rather than a
    ///   predicate over the `BETWEEN`'s result.
    fn parse_between(&mut self, left: Expr, negated: bool) -> Result<Expr> {
        self.ts.expect_kw(Keyword::Between)?;
        let outer_b_expr = std::mem::replace(&mut self.b_expr, true);
        let low = self.parse_expr_bp(BP_AND + 1);
        self.b_expr = outer_b_expr;
        let low = low?;
        self.ts.expect_kw(Keyword::And)?;
        let high = self.parse_expr_bp(BP_PRED + 1)?;
        if self.predicate_negation().is_some() {
            return Err(self.ts.syntax_error_at_or_near());
        }
        Ok(desugar_between(left, low, high, negated))
    }

    /// Parse `[NOT] LIKE`/`ILIKE pattern`. `ILIKE` is the case-insensitive
    /// variant (its own `BinaryOp`, case-folded at eval); the optional
    /// `ESCAPE` clause supports only the default `\` escape.
    fn parse_like(&mut self, left: Box<Expr>, negated: bool) -> Result<Expr> {
        let case_insensitive = self.ts.eat_kw(Keyword::Ilike)?;
        if !case_insensitive {
            self.ts.expect_kw(Keyword::Like)?;
        }
        let pattern = self.parse_expr_bp(BP_PRED + 1)?;
        // Optional `ESCAPE 'c'`. The evaluator uses the SQL default escape (`\`),
        // so an explicit `ESCAPE '\'` is redundant and accepted; any other escape
        // character is not threaded through the IR and fails loud (the spine only
        // ever passes the default).
        if self.eat_ident("escape")? {
            let escape = self.parse_expr_bp(BP_PRED + 1)?;
            if !matches!(&escape, Expr::Literal(Literal::String(s)) if s == "\\") {
                return Err(Error::NotImplemented(
                    "LIKE ESCAPE only supports the default '\\' escape character".into(),
                ));
            }
        }
        let op = match (case_insensitive, negated) {
            (false, false) => BinaryOp::Like,
            (false, true) => BinaryOp::NotLike,
            (true, false) => BinaryOp::Ilike,
            (true, true) => BinaryOp::NotIlike,
        };
        Ok(Expr::BinaryOp {
            left,
            op,
            right: Box::new(pattern),
        })
    }

    /// Parse `AT TIME ZONE zone` applied to `left` (`TIME`/`ZONE` are contextual
    /// identifiers, not reserved words).
    fn parse_at_time_zone(&mut self, left: Box<Expr>) -> Result<Expr> {
        self.ts.expect_kw(Keyword::At)?;
        self.expect_ident("time")?;
        self.expect_ident("zone")?;
        let zone = self.parse_expr_bp(BP_AT + 1)?;
        Ok(Expr::BinaryOp {
            left,
            op: BinaryOp::AtTimeZone,
            right: Box::new(zone),
        })
    }

    /// Whether the current token is a quantifier keyword (`ANY`/`ALL`/`SOME`).
    fn at_quantifier(&self) -> bool {
        self.ts.at_kw(Keyword::Any) || self.ts.at_kw(Keyword::All) || self.ts.at_kw(Keyword::Some)
    }

    /// Parse `ANY`/`ALL`/`SOME (array)`, returning `(all, array_expr, sub_select)`
    /// (`SOME` is `ANY`; `sub_select` is [`Expr::AnyAll`]'s). Assumes the quantifier
    /// keyword is current.
    fn parse_quantified(&mut self) -> Result<(bool, Expr, bool)> {
        let all = self.ts.at_kw(Keyword::All);
        self.ts.bump()?; // ANY / ALL / SOME
        // 18.6 reads `ANY ((SELECT …))` as the sub-select form, as it reads `IN`'s
        // (`sub_type select_with_parens`), and that form is not supported. The doubled
        // spelling is parsed on as an array-valued scalar subquery, and the node it builds
        // refuses when it is evaluated ([`Expr::AnyAll`]'s `sub_select`), so an error
        // raised inside it or around it before then outranks the refusal, which stands in
        // for no 18.6 error. Read as the array, `1 = ANY ((SELECT ARRAY[1, 2]))` would
        // answer `true` against 18.6's `42883`.
        let sub_select = self.at_select_with_parens();
        self.ts.expect(&TokenKind::LParen)?;
        let array = self.parse_expr_bp(0)?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok((all, array, sub_select))
    }

    /// Parse an `ARRAY[e1, e2, …]` constructor (possibly empty).
    fn parse_array(&mut self) -> Result<Expr> {
        self.ts.expect_kw(Keyword::Array)?;
        self.ts.expect(&TokenKind::LBracket)?;
        let elems = self.comma_list_until(&TokenKind::RBracket, |p| p.parse_expr_bp(0))?;
        self.ts.expect(&TokenKind::RBracket)?;
        Ok(Expr::Array(elems))
    }

    /// Parse a primary expression: a literal, column, parameter, parenthesized
    /// group, `CAST(…)`, or function call.
    ///
    /// **This is where `b_expr` resets to `a_expr`.** Every nested construct the
    /// grammar brackets — a parenthesized group, a function argument list, a `CASE`
    /// branch, an `ARRAY[…]` element, a subquery — is a `c_expr` production over
    /// full `a_expr` on both engines, and every one of them is reached from here.
    /// Resetting once, around the whole of this function, is therefore the *same*
    /// enumeration as the bracketing constructs rather than a list that can fall
    /// behind one. The mode is restored on the way out, which is what keeps
    /// `DEFAULT (now()) AT TIME ZONE 'utc'` and `DEFAULT (5 IS NULL) IS NULL`
    /// `42601` (both measured on 18.6) while `DEFAULT 1 + (NOT true)::int` is
    /// accepted — see the `b_expr` field.
    pub(super) fn parse_primary(&mut self) -> Result<Expr> {
        let outer_b_expr = std::mem::replace(&mut self.b_expr, false);
        let result = self.parse_primary_inner();
        self.b_expr = outer_b_expr;
        result
    }

    fn parse_primary_inner(&mut self) -> Result<Expr> {
        // `INTERVAL 'literal' [<qualifier>]` — `gram.y`'s
        // `ConstInterval Sconst opt_interval`. `interval` is a contextual identifier
        // (so it stays usable as a column name); only an immediately following string
        // literal makes it the interval form.
        if self.at_ident("interval") && matches!(self.ts.peek2_kind(), TokenKind::String) {
            return self.parse_interval_literal();
        }
        // Every other typed constant — `DATE '2024-06-15'`, `NUMERIC(5,2) '1.5'`,
        // `DOUBLE PRECISION '1.5'`, `myenum 'a'`. See `parse_typed_constant`.
        if let Some(expr) = self.try_typed_constant()? {
            return Ok(expr);
        }
        let kind = self.ts.peek_kind().clone();
        match kind {
            TokenKind::Number => {
                let tok = self.ts.advance()?;
                Ok(self.number_literal(&tok))
            }
            TokenKind::String => {
                let tok = self.ts.advance()?;
                Ok(Expr::Literal(Literal::String(
                    self.ts.text(&tok).to_string(),
                )))
            }
            TokenKind::Param(n) => {
                self.ts.bump()?;
                let parameter = Expr::Parameter { index: n as usize };
                // A type the client declared at `Parse` is the parameter's type for
                // the whole analysis — see `parse_single_with_param_types`.
                let declared = (n as usize)
                    .checked_sub(1)
                    .and_then(|i| self.declared_param_types.get(i).copied().flatten());
                Ok(match declared {
                    Some(target_type) => Expr::Cast {
                        expr: Box::new(parameter),
                        target_type,
                    },
                    None => parameter,
                })
            }
            TokenKind::Keyword(Keyword::True) => {
                self.ts.bump()?;
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.ts.bump()?;
                Ok(Expr::Literal(Literal::Bool(false)))
            }
            TokenKind::Keyword(Keyword::Null) => {
                self.ts.bump()?;
                Ok(Expr::Literal(Literal::Null))
            }
            TokenKind::Keyword(Keyword::Default) => {
                self.ts.bump()?;
                Ok(Expr::Default)
            }
            TokenKind::Keyword(Keyword::Cast) => self.parse_cast(),
            TokenKind::Keyword(Keyword::Array) => self.parse_array(),
            TokenKind::Keyword(Keyword::Case) => self.parse_case(),
            // `EXISTS (subquery)` only when a `(` follows. `exists` is a COL_NAME
            // keyword (a valid `ColId`), so a bare `exists` with no paren reads as
            // an ordinary column (`SELECT exists FROM t`) via the non-reserved arm.
            TokenKind::Keyword(Keyword::Exists)
                if matches!(self.ts.peek2_kind(), TokenKind::LParen) =>
            {
                self.parse_exists(false)
            }
            // String-function special forms — only when a `(` follows (paren-guarded,
            // the `EXISTS` precedent), so a bare `substring`/`trim`/`position` stays a
            // usable column name (all three are COL_NAME keywords).
            TokenKind::Keyword(Keyword::Substring)
                if matches!(self.ts.peek2_kind(), TokenKind::LParen) =>
            {
                self.parse_substring()
            }
            TokenKind::Keyword(Keyword::Trim)
                if matches!(self.ts.peek2_kind(), TokenKind::LParen) =>
            {
                self.parse_trim()
            }
            TokenKind::Keyword(Keyword::Position)
                if matches!(self.ts.peek2_kind(), TokenKind::LParen) =>
            {
                self.parse_position()
            }
            TokenKind::Keyword(Keyword::Extract)
                if matches!(self.ts.peek2_kind(), TokenKind::LParen) =>
            {
                self.parse_extract()
            }
            // A `ColId` name in expression position — a bare/quoted identifier or a
            // non-reserved keyword — is an ordinary identifier (column or function
            // name); Postgres allows e.g. `SELECT name, sequence FROM …`. The
            // explicit keyword-led productions above (`CAST`, `ARRAY`, `EXISTS`,
            // `CASE`, the literals) take precedence. Routed through the shared
            // `is_colid_name` predicate so it can't drift from the other ColId sites.
            ref k if super::is_colid_name(k) => self.parse_identifier_primary(),
            // A `TypeFuncName` keyword before `(` is a **function name**: 18.6's
            // `func_name` admits `type_function_name`, which takes that class, so
            // `left('abc', 2)` and `right(…)` are calls (as is `join(…)`, which resolves no
            // function there and here). The class is still refused as a bare column —
            // `ColId` excludes it — and in every non-leading position (`a LEFT JOIN b`,
            // `x LIKE y`) the keyword is never read here, a primary being the start of an
            // operand.
            TokenKind::Keyword(k)
                if k.pg_class() == PgClass::TypeFuncName
                    && matches!(self.ts.peek2_kind(), TokenKind::LParen) =>
            {
                self.parse_identifier_primary()
            }
            TokenKind::LParen => self.parse_paren_or_subquery(),
            _ => Err(self.ts.expected("an expression")),
        }
    }

    /// Parse a numeric literal token into an `Int` (preferred) or a `Numeric`.
    ///
    /// Never a `Literal::Float`: Postgres types a decimal/exponent constant as
    /// `numeric`, and this is the only site that turns a number token into a literal,
    /// so the variant is unreachable from **SQL text**. That is the load-bearing half and the whole
    /// of it — the variant is not unconstructed workspace-wide (the planner's
    /// `#[cfg(test)]` `Expr` serialization corpus builds one, and the executor carries
    /// a live eval arm for it), only unproducible by a parse.
    /// A literal past what the `numeric` format can hold is **not** a malformed number:
    /// 18.6's `numeric_in` answers `22003 value overflows numeric format` for it, and
    /// it answers that in parse analysis — so the refusal is deferred like an unknown
    /// operator's ([`Parser::analysis_refusal`]) and the literal reads on as a numeric
    /// zero, whose type is the `numeric` 18.6 gives the spelling. Past the cap is three
    /// facts and not one — the weight, the display scale, and the exponent 18.6's reader
    /// stops accumulating at ([`crate::types::PgNumeric::parse`]) — and all three are owed
    /// as 18.6's `22003`: `SELECT 1e2147483647` is `22003 value overflows numeric
    /// format`, not a syntax error carrying the whole exponent (measured).
    ///
    /// Neither refusal names any part of the literal: 59 B for either spelling. What
    /// stands behind that is measurement and not a reading of the grammar — a fixture
    /// per boundary cell, and a lexer test,
    /// `no_number_literal_is_refused_by_a_message_that_names_it`, which runs the 2 523
    /// short spellings the scanner admits as one number through this same parse.
    // chained if-let reads clearer than `map_or_else` here (the else branches do
    // real work).
    #[allow(clippy::option_if_let_else)]
    fn number_literal(&mut self, tok: &Token) -> Expr {
        let raw = self.ts.text(tok);
        if let Ok(i) = raw.parse::<i64>() {
            Expr::Literal(Literal::Int(i))
        } else if crate::lexer::is_undecoded_radix_literal(raw) {
            // The scanner declined the decode: the digit count alone puts the value
            // past the format cap (`lexer::Radix::decode`).
            self.owe_analysis_refusal(Error::NumericFormatOverflow);
            Expr::Literal(Literal::Numeric(crate::types::PgNumeric::zero()))
        } else {
            match crate::types::PgNumeric::parse(raw) {
                // A decimal/exponent literal (or an out-of-i64 integer) is NUMERIC in
                // Postgres. Parse it here so its exact scale round-trips.
                Ok(n) => Expr::Literal(Literal::Numeric(n)),
                Err(e) => {
                    self.owe_analysis_refusal(e);
                    Expr::Literal(Literal::Numeric(crate::types::PgNumeric::zero()))
                }
            }
        }
    }

    /// Parse a bare/quoted identifier primary: a SQL value function
    /// (`CURRENT_DATE` etc.), a function call (`name(…)`), or a (possibly
    /// qualified) column reference.
    ///
    /// The name is read through [`TokenStream::name`](super::stream::TokenStream::name),
    /// so an unquoted spelling is downcased and a quoted one is exact. That is what
    /// makes `SELECT COUNT(*)` resolve and `SELECT "COUNT"(*)` not: the second names a
    /// function called `COUNT`, which `pg_proc` does not hold (18.6: `42883`), while
    /// the quoted *lower-case* `"count"(*)` names the real one and resolves on both.
    fn parse_identifier_primary(&mut self) -> Result<Expr> {
        let first = self.ts.advance()?;
        // Whether the source wrote this word quoted. A **name** only needs the fold
        // (`ts.name`), but a value function is matched as a *keyword*, and Postgres's
        // scanner never produces a keyword from a `"…"`-quoted word — so the bit has
        // to survive one step further here than anywhere else in the parser.
        let quoted = matches!(first.kind, TokenKind::QuotedIdentifier);
        let name = self.ts.name(&first);
        // SQL value functions (`CURRENT_DATE`, `CURRENT_TIMESTAMP`, …): Postgres's
        // parenless special forms (`func_expr_common_subexpr`), recognized before
        // the function-call and column paths so a bare one never degrades to a
        // column reference. A trailing `(` is the precision variant
        // (`CURRENT_TIMESTAMP(0)`) — Postgres truncates, pglet does not model
        // sub-second precision, so it fails loud rather than silently ignoring it.
        //
        // A quoted spelling is skipped even when it matches: Postgres reads a quoted
        // **parenless** word as an ordinary column reference, so `SELECT
        // "current_timestamp"` is `42703` on 18.6 and here, and dropping this guard
        // resolves a special form the source never wrote. `from_name`'s arms are the
        // five temporal names, so the guard changes nothing for a word it does not
        // match — whatever `pg_proc` holds of that word is `build_call`'s question.
        if let Some(svf) = (!quoted)
            .then(|| SqlValueFunction::from_name(&name))
            .flatten()
        {
            if self.ts.at(&TokenKind::LParen) {
                // Most value functions take no argument list. `current_timestamp()`
                // (empty parens) and `current_date(…)` are syntax errors in
                // Postgres (`42601`); pglet also does not model the
                // `CURRENT_TIMESTAMP(precision)` variant, so a `(` after one of those
                // is a loud syntax error — never a silent precision truncation.
                //
                // **`current_schema` is the exception, and it is 18.6's**: the word is
                // a TYPE_FUNC_NAME_KEYWORD there, so it is both an `SVFOP` and
                // `pg_proc` 1402, and `current_schema()` answers `public` rather than
                // failing. That spelling falls through to `parse_function_call`, which
                // is where its arity is resolved (`current_schema(1)` → `42883`,
                // matching 18.6) — the split lives on `takes_no_argument_list`, not
                // here, so adding a member decides it on the enum.
                if svf.takes_no_argument_list() {
                    return Err(self
                        .ts
                        .error(&format!("{} takes no argument list", svf.sql_name())));
                }
                return self.parse_function_call(&name);
            }
            return Ok(Expr::Builtin(svf.builtin()));
        }
        if self.ts.at(&TokenKind::LParen) {
            return self.parse_function_call(&name);
        }
        let mut parts = vec![name];
        while self.ts.at(&TokenKind::Dot) {
            self.ts.bump()?;
            // `qualifier.*` — a qualified wildcard (`al.*`). Modeled as a column
            // named `*` carrying the qualifier; expanded to that relation's
            // columns at lowering time.
            if self.ts.at(&TokenKind::Star) {
                self.ts.bump()?;
                return Ok(Expr::Column {
                    table: Some(parts.join(".")),
                    name: "*".to_string(),
                });
            }
            let part = self.ts.advance()?;
            let part_text = match part.kind {
                // The attribute after `.` is a Postgres `ColLabel`, which admits
                // *any* keyword — including reserved ones (`c.order`, `t.from`)
                // and the `TypeFuncName` words rejected as bare columns (`t.left`).
                TokenKind::Identifier | TokenKind::QuotedIdentifier | TokenKind::Keyword(_) => {
                    self.ts.name(&part)
                }
                _ => return Err(self.ts.expected("a column name after '.'")),
            };
            // A `(` after a dotted name is a **schema-qualified function call**
            // (`pg_catalog.set_config(...)`, the pg_dump preamble): strip the
            // qualifier(s) and dispatch on the final part, mirroring how the
            // table-name path strips `public.`. `build_call` then fails loud on a
            // genuinely unknown function — but the *qualifier* is never resolved, so a
            // known function under an unknown schema answers instead of failing:
            // `SELECT nosuchschema.now() IS NOT NULL` is `t` here and `3F000` on 18.6,
            // as is `SELECT "PG_CATALOG".now()` (`"pg_catalog".now()` is `t` on both).
            // Recorded in `EDGES.md`; resolving the qualifier
            // is a gate with its own error-precedence consequences.
            if self.ts.at(&TokenKind::LParen) {
                return self.parse_function_call(&part_text);
            }
            parts.push(part_text);
        }
        column_from_parts(parts)
    }

    /// Read `func_application`'s optional leading **set quantifier**.
    ///
    /// **The enumeration is over the parsers that consume a call's argument list**, and
    /// deriving it any other way has now been wrong twice — the FROM route was missing
    /// when the quantifier was first read, and `parser::ddl`'s `parse_nextval_arg` was
    /// missing after that (while it was, `DEFAULT nextval(ALL 's')` was `42601`
    /// here against a plain accept on 18.6, while its own parenthesized spelling was
    /// accepted). Four of those parsers reach a quantifier and call this:
    /// [`Parser::parse_function_call`], `parser::select`'s `parse_unnest_table` and
    /// `parse_generate_series_table`, and `parser::ddl`'s `parse_nextval_arg` — the DDL
    /// `DEFAULT` classifier's `nextval` arm, the only arm of that classifier whose call
    /// takes an argument at all. Two more
    /// consume such a list and reach no quantifier: `parser::ddl`'s
    /// `eat_empty_call` (the classifier's six niladic arms, whose `expect(RParen)` *is*
    /// 18.6's grammar bound — `DEFAULT now(ALL)` is `42601` on both) and the five special
    /// forms below.
    ///
    /// The reading routes in full: this file's [`Parser::parse_function_call`] and
    /// `parser::select`'s `parse_unnest_table` and `parse_generate_series_table` (the FROM
    /// clause's functions — `parse_table_factor` dispatches a derived table, those two, a
    /// CTE reference, a catalog view and a base table, and nothing else, so
    /// `FROM upper(ALL 'a')` is `42601` here against rows on 18.6, exactly as its
    /// unquantified spelling already was: a FROM-clause call on a function pglet models
    /// elsewhere is refused only because `parse_table_factor` has no arm for it, and this
    /// comment is the only place that is recorded). A quantifier can be *written* on the
    /// special forms too — `parse_substring` / `parse_trim` / `parse_position` /
    /// `parse_extract` / `parse_cast` — and **four of the five** refuse it as a syntax
    /// error where 18.6 does too, which is why they read nothing. `parse_extract` is the
    /// exception, and its divergence is not the quantifier's: its `is_field_word` admits
    /// `TokenKind::Keyword(_)`, so **any** reserved word standing before `FROM` is taken
    /// as the field and resolved at eval, where 18.6's `extract_arg` production is
    /// `IDENT | YEAR_P | … | Sconst` and refuses it in bison. `EXTRACT(ALL FROM now())`
    /// and its `DISTINCT` spelling are `42601` there against
    /// `22023 unit "all" not recognized` here, and so are `EXTRACT(SELECT FROM now())`
    /// and `EXTRACT(TABLE FROM now())` — the two that identify the class as the field
    /// grammar rather than the quantifier. Only the spelling standing *directly* before
    /// `FROM` reaches it: `EXTRACT(ALL year FROM now())` is `42601` on both, from the
    /// comma form's own expression parse. Pre-existing, recorded on `docs/compatibility.md`'s
    /// `extract` row, and untouched by the quantifier rules.
    /// The read is one function because the
    /// *verdicts* differ by route and the token grammar must not.
    ///
    /// Two quantifiers (`count(ALL DISTINCT a)` and its mirror) need no rule of their
    /// own: both keywords are reserved, so the second one reaches `parse_expr_bp` and
    /// is `42601` there, as on 18.6 (measured on both, in both orders, at all three
    /// routes).
    pub(super) fn parse_func_set_quantifier(&mut self) -> Result<SetQuantifier> {
        if self.ts.eat_kw(Keyword::Distinct)? {
            Ok(SetQuantifier::Distinct)
        } else if self.ts.eat_kw(Keyword::All)? {
            Ok(SetQuantifier::All)
        } else {
            Ok(SetQuantifier::None)
        }
    }

    /// `gram.y`'s bound on a set quantifier: both quantifier productions require a
    /// *non-empty* `func_arg_list`, and the bare `func_name '(' '*' ')'` production
    /// combines with neither — so a quantifier beside `)` or beside `*` is a syntax
    /// error (`42601`) there. `empty` is the caller's spelling of "there is no argument
    /// list here".
    ///
    /// The bound is stated in the parser and nowhere else: it is the grammar's, and
    /// `planner::build_call` accordingly never sees a quantifier beside `wildcard`. Its
    /// spellings are the cross-product **{`DISTINCT`, `ALL`} × {`)`, `*`} × {ordinary
    /// call, window call}**, each measured `42601` on 18.6 and each now `42601` here:
    /// `upper(DISTINCT)` / `upper(ALL)` / `now(DISTINCT)` / `now(ALL)`,
    /// `upper(DISTINCT *)` / `upper(ALL *)` / `count(DISTINCT *)` / `count(ALL *)`,
    /// `row_number(DISTINCT) OVER ()` / `row_number(ALL) OVER ()`,
    /// `row_number(DISTINCT *) OVER ()` / `row_number(ALL *) OVER ()`. Four of those
    /// **answered rows** here before — `now(DISTINCT)`, `count(DISTINCT *)`,
    /// `row_number(DISTINCT) OVER ()` and `row_number(DISTINCT *) OVER ()`, the last two
    /// because the window fork then read neither `args` nor `wildcard` — and the rest were
    /// loud with the wrong code. Which spelling was silent and which was loud is a fact
    /// about the *name*'s dispatch rather than about the cell, so the cells are not
    /// uniform: `upper(DISTINCT)` was `42883` and `upper(DISTINCT *)` was `0A000`.
    /// The FROM-clause `unnest` route shares this bound and has no window or `*` half:
    /// `FROM unnest(ALL) AS u(x)` and its `DISTINCT` twin are `42601` on both engines.
    ///
    /// # Errors
    ///
    /// `Error::Syntax` when a quantifier is written and `empty`.
    pub(super) fn bound_func_set_quantifier(
        &self,
        quantifier: SetQuantifier,
        empty: bool,
    ) -> Result<()> {
        if let Some(keyword) = quantifier.keyword()
            && empty
        {
            return Err(self.ts.error(&format!(
                "{keyword} in a function call must be followed by an argument list"
            )));
        }
        Ok(())
    }

    /// Parse a function call `name(args)`, dispatching by name to the typed
    /// [`Expr`] variant.
    ///
    /// The optional leading set quantifier is read by [`Parser::parse_func_set_quantifier`]
    /// and bounded by [`Parser::bound_func_set_quantifier`]; the verdict on the quantifier
    /// itself is `planner::build_call`'s.
    fn parse_function_call(&mut self, name: &str) -> Result<Expr> {
        self.ts.expect(&TokenKind::LParen)?;
        let quantifier = self.parse_func_set_quantifier()?;
        let mut wildcard = false;
        let args = if self.ts.at(&TokenKind::Star) {
            self.ts.bump()?; // `COUNT(*)`
            wildcard = true;
            Vec::new()
        } else {
            self.comma_list_until(&TokenKind::RParen, |p| p.parse_expr_bp(0))?
        };
        // `wildcard` implies an empty `args` today, so the first disjunct changes no
        // answer and a mutation deleting it alone is a no-op (measured). It is spelled
        // because the two are **different grammar facts** — `func_name '(' '*' ')'`
        // carries no `func_arg_list` at all, where the other spelling carries an empty
        // one — so a future wildcard representation that parsed a node would otherwise
        // drop the `*` half silently. The half that *is* measurable is the pair: the
        // spelling `!wildcard && args.is_empty()` reddens three fixtures.
        self.bound_func_set_quantifier(quantifier, wildcard || args.is_empty())?;
        self.ts.expect(&TokenKind::RParen)?;
        // A trailing `OVER (…)` makes this a window function (`OVER` is a contextual
        // identifier, not a reserved keyword); otherwise dispatch the ordinary call.
        //
        // The window branch judges its argument list, not its quantifier — 18.6 decides
        // both quantifier spellings by the *argument* rather than the modifier
        // (`row_number(DISTINCT 1) OVER ()` is `42883 function row_number(integer) does
        // not exist` there, `dense_rank(DISTINCT a) OVER (…)` is `42809 WITHIN GROUP is
        // required …`), which is `planner::refuse_window_arguments`' rule. The bound
        // above still applies to it: `row_number(DISTINCT) OVER ()` is `42601`.
        if self.at_ident("over") {
            return self.parse_window_over(name, quantifier, &args);
        }
        crate::planner::build_call(name, args, quantifier, wildcard)
    }

    /// Parse a window function's `OVER ( [PARTITION BY …] [ORDER BY …] )` clause and
    /// build an [`Expr::Window`]. The supported functions are the niladic
    /// `ROW_NUMBER`/`RANK`/`DENSE_RANK`; a frame clause is not modeled.
    ///
    /// The call's argument list is judged once the `OVER` clause is read, where 18.6
    /// judges it: a syntax error anywhere in the clause outranks the verdict, as 18.6's
    /// raw parse outranks its analysis, and the verdict outranks every name the clause's
    /// keys bind, which 18.6 transforms only after the call resolves —
    /// `row_number(1) OVER (PARTITION BY nosuch)` is `42883` on both.
    fn parse_window_over(
        &mut self,
        name: &str,
        quantifier: SetQuantifier,
        args: &[Expr],
    ) -> Result<Expr> {
        // `name` came from `parse_identifier_primary`, already downcased if unquoted,
        // so this compares exactly: `"ROW_NUMBER"() OVER ()` names a function no
        // catalog holds (18.6: `42883`).
        let func = match name {
            "row_number" => WindowFunc::RowNumber,
            "rank" => WindowFunc::Rank,
            "dense_rank" => WindowFunc::DenseRank,
            _ => {
                return Err(Error::NotImplemented(format!(
                    "window function not supported: {name}"
                )));
            }
        };
        self.expect_ident("over")?;
        self.ts.expect(&TokenKind::LParen)?;
        let partition_by = if self.eat_ident("partition")? {
            self.ts.expect_kw(Keyword::By)?;
            self.parse_expr_list()?
        } else {
            Vec::new()
        };
        let order_by = if self.ts.eat_kw(Keyword::Order)? {
            self.ts.expect_kw(Keyword::By)?;
            self.parse_order_by()?
        } else {
            Vec::new()
        };
        self.ts.expect(&TokenKind::RParen)?;
        crate::planner::refuse_window_arguments(name, func, quantifier, args)?;
        Ok(Expr::Window {
            func,
            partition_by,
            order_by,
        })
    }

    /// Parse `CAST(expr AS type)`.
    fn parse_cast(&mut self) -> Result<Expr> {
        self.ts.expect_kw(Keyword::Cast)?;
        self.ts.expect(&TokenKind::LParen)?;
        let inner = self.parse_expr_bp(0)?;
        self.ts.expect_kw(Keyword::As)?;
        let cast = self.parse_cast_target(Box::new(inner))?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(cast)
    }

    /// Parse a parenthesized expression. A scalar subquery `(SELECT …)` is
    /// deferred to the SELECT parser and currently fails loud.
    fn parse_paren_or_subquery(&mut self) -> Result<Expr> {
        self.ts.expect(&TokenKind::LParen)?;
        if self.ts.at_kw(Keyword::Select) {
            let plan = self.in_expr_subquery(Self::parse_select)?;
            self.ts.expect(&TokenKind::RParen)?;
            return Ok(Expr::Subquery {
                plan: Box::new(plan),
            });
        }
        if self.ts.at_kw(Keyword::With) || self.ts.at_kw(Keyword::Values) {
            return Err(Error::NotImplemented(
                "WITH / VALUES subquery not yet supported in the native parser".into(),
            ));
        }
        let inner = self.parse_expr_bp(0)?;
        self.ts.expect(&TokenKind::RParen)?; // parens are structural — no wrapper
        Ok(inner)
    }

    /// Resolve an already-parsed **canonical** type name for a cast target.
    ///
    /// The name comes from the **same** reader as the DDL column path
    /// ([`Parser::parse_type_name`]) and goes to the same enum-aware resolver
    /// ([`crate::executor::resolve_column_type`]), so the `::`/`CAST` and
    /// `CREATE TABLE` type grammars can't drift — this is what makes
    /// `CAST(x AS double precision)` / `CAST(x AS varchar(255))` parse (the old
    /// single-identifier reader left the trailing word/length behind). A user
    /// `ENUM` resolves to `Text` plus its name; an unrecognized or unimplemented type
    /// fails loud (`42704`).
    fn resolve_type_with_typmod(
        &self,
        canonical: &str,
    ) -> Result<crate::executor::ResolvedColumnType> {
        crate::executor::resolve_column_type(canonical, self.schema)
    }

    /// Parse a cast target and build the cast over `inner`, routing the bare spelling
    /// `regclass` to its own node.
    ///
    /// `regclass` is not a pglet [`TypeOid`] and deliberately is not becoming one: 18.6's
    /// `regclass` value is an OID that *prints* as a relation name, and this engine's
    /// encoders carry no catalog to resolve one with, so such a value could render as
    /// 18.6 renders it in the text format or in the binary one and not both. It lowers
    /// instead to [`StringFunc::Regclass`], which answers the relation's name **rendered
    /// for an identifier position** ([`crate::lexer::quote_ident`], so `"Odd"` reads back
    /// with its quotes) as `text`, encoded by the `text` codecs like any other string.
    ///
    /// **The branch is decided on the canonical type name, never on the leading
    /// identifier.** [`Parser::parse_type_name`] also consumes a trailing `[]` (one or
    /// more, with an optional dimension) and a `(…)` modifier, so a reader that peeked
    /// the word `regclass` and discarded that reader's return built the node
    /// `'t'::regclass` builds for `'t'::regclass[]`, `'t'::regclass(3)` and
    /// `CAST('t' AS regclass[])` alike — answering a relation name where 18.6 raises,
    /// and answering it inside a `WHERE`, which **wrote**. Comparing the whole
    /// canonical spelling routes every other one to
    /// [`Self::resolve_type_with_typmod`], where `regclass[]` and `regclass(3)` get the
    /// `42704 type "regclass" is not supported` the *column* form already answers.
    ///
    /// Where the surviving bare spelling lands beside 18.6 is **recorded rather than
    /// bounded here**: `EDGES.md` row 85 is the record for it, row 84 for the
    /// stored-expression refusal, row 118 for the spellings this branch now refuses.
    fn parse_cast_target(&mut self, inner: Box<Expr>) -> Result<Expr> {
        let canonical = self.parse_type_name()?;
        // `expect_type_word` takes a bare `TokenKind::Identifier` only and downcases
        // it, and `regclass` reaches `parse_scalar_type_name`'s pass-through arm, so this
        // string is `regclass` exactly when the written target was the bare word in any
        // case. A `"regclass"`-quoted spelling never arrives (`42601` here against `t` on
        // 18.6, measured — `EDGES.md` row 118), and a suffix or a modifier makes the
        // canonical name a different string.
        if canonical == "regclass" {
            return Ok(Expr::StringFn {
                func: StringFunc::Regclass,
                args: vec![*inner],
            });
        }
        let resolved = self.resolve_type_with_typmod(&canonical)?;
        Ok(self.build_resolved_cast(inner, &resolved))
    }

    /// Build the cast of `inner` to a resolved target — the one builder behind the `::`,
    /// `CAST(… AS …)` and typed-constant (`mq 'a'`) spellings.
    ///
    /// A target that resolves to a user enum takes [`crate::planner::build_enum_cast`]
    /// over the member list the catalog holds now, so each of the three spellings converts
    /// to an enum by one rule; every other target is [`Self::build_cast`]. The enum's
    /// `typmod` is `None` by construction — [`crate::executor::resolve_column_type`]
    /// refuses a modifier an enum does not carry.
    fn build_resolved_cast(
        &self,
        inner: Box<Expr>,
        resolved: &crate::executor::ResolvedColumnType,
    ) -> Expr {
        if let Some(enum_type) = resolved
            .enum_type
            .as_deref()
            .and_then(|name| self.schema.enums.get(name).map(|e| (name, e)))
        {
            return crate::planner::build_enum_cast(*inner, enum_type.0, &enum_type.1.labels);
        }
        Self::build_cast(inner, resolved.type_oid, resolved.typmod())
    }

    /// Build a cast expression over `inner`: [`Expr::CastTypmod`] when the target
    /// carried a modifier, else a plain [`Expr::Cast`]. Shared by the `::` and
    /// `CAST(… AS …)` forms so both enforce typmods identically.
    const fn build_cast(
        inner: Box<Expr>,
        type_oid: TypeOid,
        typmod: Option<crate::planner::Typmod>,
    ) -> Expr {
        match typmod {
            Some(typmod) => Expr::CastTypmod {
                expr: inner,
                type_oid,
                typmod,
            },
            None => Expr::Cast {
                expr: inner,
                target_type: type_oid,
            },
        }
    }
}

/// Build a (possibly qualified) column reference from dotted identifier parts.
///
/// One part → a bare column; two → `table.column`. A reference with **more than
/// two** parts (`a.b.c` — a schema/catalog qualifier) **fails loud**: pglet has a
/// single implicit schema and doesn't model deeper qualification, so silently
/// truncating to the last part (the prior behavior) could mis-resolve.
fn column_from_parts(mut parts: Vec<String>) -> Result<Expr> {
    match parts.len() {
        2 => {
            let name = parts.pop().unwrap_or_default();
            let table = parts.pop();
            Ok(Expr::Column { table, name })
        }
        n if n > 2 => Err(Error::NotImplemented(format!(
            "qualified column reference with more than two parts is not supported: {}",
            parts.join(".")
        ))),
        _ => {
            let name = parts.pop().unwrap_or_default();
            Ok(Expr::Column { table: None, name })
        }
    }
}

/// Apply unary `-` to an already-parsed operand, **folding it into the constant**
/// when the operand is a numeric constant — Postgres's `doNegate` (`gram.y`).
///
/// `doNegate` inspects its operand and negates *in place* for exactly two `A_Const`
/// kinds — an `Integer` (`Iconst`) and a `Float` (`FCONST`: a decimal, an exponent
/// form, or an integer too wide for the `Integer` node) — and builds an ordinary
/// `-` operator node for everything else. The two kinds map onto
/// [`Literal::Int`] and [`Literal::Numeric`], which are the only two
/// [`Parser::number_literal`] produces; a
/// [`Literal::Float`] cannot reach here (the parser builds none, so no SQL text
/// produces one — see that function's note), and a string / boolean / `NULL` constant
/// is **not** one of
/// `doNegate`'s two kinds, so each keeps its operator node exactly as there
/// (`SELECT -'1'` is `42725` on 18.6, not a constant `-1`).
///
/// **The fold is what makes a negative ordinal reachable at all**, which is the
/// whole reason it is not cosmetic. `findTargetlistEntrySQL92` reads a bare
/// `ORDER BY` / `GROUP BY` / `DISTINCT ON` key as a **position** exactly when it is
/// an `A_Const`, so with the fold `ORDER BY -1` is `42P10 position -1 is not in
/// select list` and `ORDER BY -1.5` is `42601 non-integer constant in ORDER BY`,
/// while without it the key is an operator node that sorts by a constant — a no-op
/// the statement then *runs*. That silently ran a `DELETE`/`UPDATE` whose filtering
/// subquery carried such a key, on a statement 18.6 refuses outright, so the rows
/// it removed were removed against an error (guarded in
/// `crates/pglet/tests/scope_relation_identity.rs`).
///
/// A **no-FROM `SELECT`** used to be exempt from all of that, and it was the discard
/// rather than the fold: `parse_select_inner`'s `SelectLeaf::NoFrom` arm parsed the
/// sort/bound tail and dropped it, so the bounds check never ran in either sign
/// (`SELECT 1 AS q ORDER BY -1` and its positive control `ORDER BY 5` both answered a row
/// against 18.6's `42P10`) and the silent write survived with it
/// (`DELETE FROM n1 WHERE a IN (SELECT 1 AS q ORDER BY -1)` was `DELETE 1` against 18.6's
/// refusal and three surviving rows). The tail routes through
/// [`plan_no_from_order_limit`](crate::planner::plan_no_from_order_limit) now and the
/// exemption is gone, both signs and both polarities — guarded by
/// `select/no_from_order_by_negative_position_loud.sql` beside its positive control, and
/// by `a_no_from_sort_key_out_of_range_position_does_not_delete` for the rows.
///
/// What is folded and what is not is decided by the **operand node**, not by the
/// source text, and the two disagree in both directions — each measured on 18.6:
///
/// - `-(1)` folds (parens are structural in `c_expr`, exactly as here) → `42P10`;
/// - `- -1` folds twice → the ordinal `1`, and `- -99` is `42P10 position 99`;
/// - `-1::int` does **not** fold: `TYPECAST` outranks `UMINUS` there (and
///   `BP_CAST > BP_UNARY` here), so the operand is a cast node → an ordinary sort key;
/// - `-abs(1)`, `-id`, `-$1` do not fold — a function call, a column, a parameter.
///
/// Unary `+` is deliberately **not** folded, here or there
/// ([`Expr::UnaryPlus`](crate::planner::Expr::UnaryPlus)): `'+' a_expr` stays an
/// operator node in `gram.y`, which is why `ORDER BY +1` is not an ordinal on either
/// engine.
///
/// Three consequences reach past the clause rules, all measured.
///
/// - **A negative literal becomes an index probe.** `is_row_independent` accepts a bare
///   literal / `$N` / bound value and not an `Expr::Negate`, so `WHERE indexed = -1` used
///   to decline the index and row-filter; folded, it probes like `1` does. The
///   substituted probe is *faithful* —
///   it decides every stored key exactly as the row filter it replaces would, which is the
///   whole of what this fold owes (the exact-or-decline gate `probe_key_value`, in
///   `executor::query::scan`) — and `index/eq_negative_literal.sql` /
///   `index/range_negative_literal.sql` carry `@profile-lock` for that, with the plan
///   *shape* (which no differential fixture can see) pinned by `planner::tests`'
///   `a_folded_negative_literal_is_a_probe_where_unary_plus_is_not`. `+1` still declines,
///   as [`Expr::UnaryPlus`](crate::planner::Expr::UnaryPlus) records.
///
///   A faithful probe is **not** profile equality: one measured build-profile
///   exception class has a member a folded negative probe reaches, through *this*
///   spelling over an ordinary `int4` key: a
///   probe that matches no row starves a leaf beside it that pglet refuses only when it is
///   evaluated, so over `ti(a INT, v TEXT)` with `CREATE INDEX ti_a ON ti (a)` holding
///   `(1, 'x')`, `SELECT v FROM ti WHERE a = -1 AND DEFAULT IS NULL` is `(0 rows)` on the
///   accelerated build against `42601` stripped and on 18.6 — measured on distinct release
///   binaries, and `42601` on both profiles once the probe *matches* (`a = 1`)
///   (`EDGES.md` row 56, `docs/indexes.md` § The starved-leaf exception).
///
///   **A container key has no literal a sign folds into**, so the fold reaches no
///   `jsonb` or array key: a written container value is a `Cast` or an `Expr::Array`,
///   which index selection does not lift, and a container's *equality* probe reaches
///   its index only through a bound `$N`. A **float** key's literal folds, and a folded
///   negative literal probes it as a positive one does: over `fk8 (x FLOAT8)` under an index on `x`,
///   `WHERE x = -0.0` finds the rows holding `0` and `-0` on both profiles and on 18.6
///   (`float_key/eq_zero_finds_both_signs.sql`).
/// - **A negative constant's declared type is decided on the signed value**, which fixes
///   a divergence rather than causing one: `-2147483648` fits `int4` and 18.6 declares
///   `integer`, where the wrapper took its type from the unsigned `2147483648` and
///   declared `int8` (`select/negative_literal_declared_types.sql`).
/// - **A stored expression persists the folded constant**, so a CHECK predicate or index
///   key written as `> -1` now holds a `Literal::Int(-1)` where it held
///   `Negate(Literal::Int(1))`. **No store-magic bump**, and the enumeration behind that:
///   the `Negate` variant still exists and still evaluates identically, so a record
///   written before this decodes and answers the same; the dump renders either shape to
///   SQL both engines reload (`(- 1)` and `-1`, `test_dump`'s `expr_probe` carries the
///   folded form); and the only structural comparisons of a persisted `Expr` are
///   within-statement (the output-entry identity, the `DISTINCT ON` merge) or
///   shape-restricted to `IS [NOT] NULL` (`query_implies_index_predicate`) — none reads a
///   persisted expression against a freshly parsed one. What a pre-fold store loses is
///   `dump`'s `parse(render(e)) == e` property for that one shape — `Negate(Int(1))`
///   renders `(- 1)` and re-parses to `Literal::Int(-1)` — and nothing else: no meaning
///   moved, unlike the `PGMD`/`PGMH` bumps, and the round-trip corpus is unaffected
///   because its `Negate` entry negates a **column** (`(- a) < 0`), which never folded.
///   No SQL text can produce the legacy shape any more, so a fresh store cannot hold it.
///
/// The `Int` arm negates with `checked_neg` and falls back to the operator node on
/// overflow. That arm is unreachable from SQL text — the only `i64` without a
/// negation is `i64::MIN`, and its positive spelling `9223372036854775808` exceeds
/// `i64` and parses to [`Literal::Numeric`] — so the fallback exists to keep the
/// parser total rather than to be reached. (`-9223372036854775808` therefore stays a
/// `Numeric` and is declared `numeric` where 18.6 says `bigint` — a pre-existing
/// literal-width divergence the fold neither causes nor closes.)
fn negate(operand: Expr) -> Expr {
    match operand {
        Expr::Literal(Literal::Int(n)) => n.checked_neg().map_or_else(
            || Expr::Negate(Box::new(Expr::Literal(Literal::Int(n)))),
            |negated| Expr::Literal(Literal::Int(negated)),
        ),
        // `PgNumeric::negate` leaves zero unsigned (there is no negative zero), which
        // is 18.6's answer too: `SELECT -0.0` renders `0.0` and `0.0 = -0.0` is true.
        Expr::Literal(Literal::Numeric(n)) => Expr::Literal(Literal::Numeric(n.negate())),
        other => Expr::Negate(Box::new(other)),
    }
}

/// Whether `op` admits a quantified `ANY`/`ALL`/`SOME` right operand (i.e. is a
/// comparison operator).
const fn is_comparison(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge,
    )
}

/// The expression the operator loop is building, and how deep it is.
///
/// Each postfix or infix arm puts everything built so far under one new node, so a
/// long flat chain — `1 + 1 + …`, `x::int::int…`, `a IS NOT NULL IS NOT NULL …` —
/// builds a tree as deep as the chain is long without the parser recursing once, and
/// every later walker recurses over it. [`Self::wrap`] measures each new node against
/// the chain's known depth (never re-walking the chain) and refuses one deeper than
/// the thread's limit ([`crate::depth`]) the moment it is built, so no walker — and
/// no `Drop` of the refused tree — ever sees more than that.
///
/// A run of one `AND`/`OR` is gathered into a list instead ([`Self::push_bool`]) and
/// built by [`Expr::bool_list`] when the run ends: 18.6's `makeAndExpr` /
/// `makeOrExpr` flatten a chain into one n-ary node on sight, which is why it answers
/// a chain of any length, and a long list is built balanced here for the same reason.
///
/// A chain past the limit is **not** an error here: it is replaced by a `NULL`
/// placeholder and the parse goes on, so an error 18.6 raises before analysis later in
/// the statement still outranks the `54001` it owes ([`super::Parser::depth_refusal`],
/// `Parser::refusal_answer`).
struct OperatorChain {
    left: Expr,
    /// The parser's nesting where this chain is being built: its frames are on the
    /// stack while any walker runs over the chain during the parse, so they count
    /// against the same limit.
    nesting: usize,
    /// `left`'s depth, once something has needed it.
    depth: Option<usize>,
    /// The `AND`/`OR` run being gathered: its connective and its operands so far.
    run: Option<(BinaryOp, Vec<Expr>)>,
    /// Whether the chain outgrew the limit and was replaced by a placeholder.
    refused: bool,
}

impl OperatorChain {
    const fn new(left: Expr, nesting: usize) -> Self {
        Self {
            left,
            nesting,
            depth: None,
            run: None,
            refused: false,
        }
    }

    /// Keep `node`, `depth` levels deep, as the chain — or, past the thread's limit,
    /// drop it for a one-level placeholder and record the refusal.
    fn settle(&mut self, node: Expr, depth: usize) {
        if crate::depth::check(depth + self.nesting).is_ok() {
            self.left = node;
            self.depth = Some(depth);
        } else {
            self.left = Expr::Literal(Literal::Null);
            self.depth = Some(1);
            self.refused = true;
        }
    }

    /// The chain so far, for an operator that will not be built: the refusal it owes is
    /// recorded against it, and [`Self::refuse_operand`] replaces it.
    fn take_operand(&mut self) -> Expr {
        self.close_run();
        std::mem::replace(&mut self.left, Expr::Default)
    }

    /// Replace the chain with a one-level placeholder, where the statement is already
    /// refused for what the operator was (`Parser::owe_unknown_operator`) — the same
    /// placeholder a chain past the depth limit leaves, and for the same reason: the
    /// statement parses on, and nothing it builds is ever executed or stored.
    fn refuse_operand(&mut self) {
        self.left = Expr::Literal(Literal::Null);
        self.depth = Some(1);
    }

    /// Put the chain under the node `arm` builds around it, settling the result against
    /// the thread's limit ([`Self::settle`]).
    fn wrap(&mut self, arm: impl FnOnce(Box<Expr>) -> Result<Expr>) -> Result<()> {
        self.close_run();
        let known_depth = match self.depth {
            Some(depth) => depth,
            None => crate::depth::expr_depth(&self.left),
        };
        // Boxed here rather than by the arm, so its address is known before the arm
        // puts it under the new node.
        let left = Box::new(std::mem::replace(&mut self.left, Expr::Default));
        let known: *const Expr = &raw const *left;
        let node = arm(left)?;
        #[cfg(test)]
        super::operator_loop_arms::note(&node);
        let depth = crate::depth::expr_depth_over(&node, known, known_depth);
        self.settle(node, depth);
        Ok(())
    }

    /// Add `right` to the `op` run, starting one — with the chain so far as its first
    /// operand, flattened when it is itself an `op` list, as `makeAndExpr` does — if
    /// none is open or the open one is the other connective.
    fn push_bool(&mut self, op: BinaryOp, right: Expr) {
        if !matches!(&self.run, Some((run_op, _)) if *run_op == op) {
            self.close_run();
            let first = std::mem::replace(&mut self.left, Expr::Default);
            let mut flattened = Vec::new();
            first.bool_operands(op, &mut flattened);
            let operands = if flattened.len() > 1 {
                // Only a parenthesized `op` list as the first operand gets here.
                flattened.into_iter().cloned().collect()
            } else {
                vec![first]
            };
            self.run = Some((op, operands));
        }
        if let Some((_, operands)) = &mut self.run {
            operands.push(right);
        }
    }

    /// Build the open `AND`/`OR` run, if any, into the chain.
    fn close_run(&mut self) {
        if let Some((op, operands)) = self.run.take() {
            let node = Expr::bool_list(op, operands);
            #[cfg(test)]
            super::operator_loop_arms::note(&node);
            let depth = crate::depth::expr_depth(&node);
            self.settle(node, depth);
        }
    }

    /// The finished expression, and whether the chain was refused — measured here if
    /// nothing above did, since a construct nested inside it deepens it too
    /// (`abs(abs(…))` builds one level per call without the loop wrapping at all).
    fn finish(mut self) -> (Expr, bool) {
        self.close_run();
        if self.depth.is_none() {
            let left = std::mem::replace(&mut self.left, Expr::Default);
            let depth = crate::depth::expr_depth(&left);
            self.settle(left, depth);
        }
        (self.left, self.refused)
    }
}

/// Whether `kind` is a predicate-band keyword (`IN`/`BETWEEN`/`LIKE`/`ILIKE`).
const fn is_pred_keyword(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Keyword(Keyword::In | Keyword::Between | Keyword::Like | Keyword::Ilike),
    )
}

/// Desugar `[NOT] BETWEEN low AND high` into comparisons: `val >= low AND val <= high`,
/// or `val < low OR val > high` when negated.
fn desugar_between(val: Expr, low: Expr, high: Expr, negated: bool) -> Expr {
    if negated {
        binop(
            binop(val.clone(), BinaryOp::Lt, low),
            BinaryOp::Or,
            binop(val, BinaryOp::Gt, high),
        )
    } else {
        binop(
            binop(val.clone(), BinaryOp::Ge, low),
            BinaryOp::And,
            binop(val, BinaryOp::Le, high),
        )
    }
}

/// Build a binary-operator `Expr`.
/// Whether `word` can begin a **multi-word or modifier-bearing** type name — the
/// only typed-constant shape two tokens of lookahead cannot decide, and therefore
/// the only one `Parser::try_typed_constant` speculates on.
///
/// The set is the lead words of `gram.y`'s `Numeric` / `ConstBit` /
/// `ConstCharacter` / `ConstDatetime` productions (the ones with a `PRECISION` /
/// `VARYING` / `WITH TIME ZONE` tail or a `( … )` modifier), plus the
/// modifier-taking spellings `parser::type_name` recognizes. It is deliberately
/// *not* "any identifier followed by `(`": that would speculate on every function
/// call, and a generic type with a modifier is `42601 type modifier is not allowed
/// for type "int4"` on 18.6 rather than a value.
///
/// Matching is ASCII-case-insensitive against a bare identifier's source spelling,
/// the width `Parser::at_ident` uses.
///
/// **A word belongs here only if `parse_scalar_type_name` can actually read the type
/// it leads.** `national` was in the set and is not any more: nothing reads
/// `NATIONAL CHARACTER [VARYING]`, so its speculation could only ever fail and
/// restore, which is a lead word that makes the set look complete when it is not
/// (`SELECT national character '5'` is a value on 18.6 and `42601` here — recorded,
/// not closed). `dec` stayed and gained its type instead: it is `decimal`'s SQL
/// abbreviation, one arm in the type-name reader, and it fixes `x::dec` with it.
fn is_multiword_type_lead(word: &str) -> bool {
    [
        "double",
        "character",
        "numeric",
        "decimal",
        "dec",
        "varchar",
        "char",
        "bpchar",
        "float",
        "bit",
        "varbit",
        "time",
        "timestamp",
        "timestamptz",
        "timetz",
    ]
    .iter()
    .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

/// Whether an [`is_multiword_type_lead`] word followed by `(` leaves the speculative
/// parse **no decline** — the window where a failure is the statement's answer rather
/// than a signal to put the cursor back.
///
/// `float` is the member, because its `(…)` is the one modifier that *selects* a type:
/// `Parser::float_type_name` raises `22023` inside the speculation, and a restore would
/// swallow that verdict and demote it to whatever the function-call path says next —
/// the failure the phase split above already records for `time with time zone`'s
/// `42704`. Committing is what makes `SELECT float(0)` this `22023` rather than pglet's
/// own `0A000 function not supported: float`, and it takes the malformed spellings with
/// it (`float(1.5)`, `float(1,2)`, `float(2147483648)` are `42601` on both engines).
///
/// **What decides the window is the `(`, not the lead word**, and the bare-word control
/// is what says so: `SELECT float` is a column reference on both engines
/// (`42703 column "float" does not exist`), so the speculation must still restore
/// there. 18.6 has the same split for the same reason — `float` is a
/// `COL_NAME_KEYWORD` (`kwlist.h`), which `func_name`'s `type_function_name` excludes,
/// so `float (` can begin nothing but `Numeric`, while bare `float` reduces to `ColId`
/// and a column reference.
///
/// **The set is narrower than that reason reaches**, deliberately. The membership test
/// is `kwlist.h`'s category, and the [`is_multiword_type_lead`] words `character`,
/// `numeric`, `char`, `varchar`, `bit`, `time`, `timestamp`, `dec` and `decimal` pass it
/// too, each `42601 syntax error at end of input` on 18.6 at `SELECT <word>(3)`,
/// measured. `interval` and `nchar` pass that test too and are not lead words here at
/// all, so their modifier-bearing spelling never reaches this window:
/// `SELECT interval(3) '1 day'` is `1 day` on 18.6 and `SELECT nchar(3) 'ab'` is `ab `,
/// each `0A000` here. This set is pglet's speculation list, not the class. Those
/// divergences are their own, tracked rather than closed here (`EDGES.md` row 116). The
/// five lead words that *fail* it — `bpchar`, `varbit`, `timestamptz`, `timetz` and the
/// `UNRESERVED_KEYWORD` `double` — must **never** join: `SELECT bpchar(3)` is `3` on
/// 18.6 and the other four are `42883 function … does not exist`, all function-call
/// answers, so a commit there would answer a type-name parse error where 18.6 still
/// reads a function call — the direction 18.6 does not go. Each of the five already
/// diverges here by its own `0A000 function not supported: <word>`, measured.
const fn commits_on_open_paren(word: &str) -> bool {
    word.eq_ignore_ascii_case("float")
}

/// One `opt_interval` field word — the six single-field qualifiers of an
/// `INTERVAL 'literal' <field>` constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntervalField {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

impl IntervalField {
    /// The unit word [`PgInterval::parse`] reads, which is how a bare number takes
    /// this field's unit (see [`interval_from_unit`]).
    const fn sql_word(self) -> &'static str {
        match self {
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Minute => "minute",
            Self::Second => "second",
        }
    }
}

/// Whether the literal is a **bare signed decimal number** — no unit words, no
/// colons, no `-`-separated fields.
///
/// This is the case `DecodeInterval`'s `range` argument governs: a number with no
/// unit of its own takes the qualifier's unit. A literal that names its own units
/// (`'1 day 3 hours'`) is decoded by the ordinary grammar and only masked.
fn is_bare_interval_number(literal: &str) -> bool {
    let body = literal.trim();
    let digits = body.strip_prefix(['+', '-']).unwrap_or(body);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.chars().filter(|c| *c == '.').count() <= 1
        && digits.chars().any(|c| c.is_ascii_digit())
}

/// A bare number read in `unit`, before masking — `DecodeInterval`'s treatment of an
/// unqualified numeric field under a range.
///
/// **It re-decodes rather than computing**: `INTERVAL '90' MINUTE` is handed to
/// [`PgInterval::parse`] as `90 minute`, which is the spelling `DecodeInterval`
/// reduces it to and the one decoder this engine already has. Doing the unit
/// arithmetic here instead was a second implementation of it, and it diverged from
/// the first in both directions — an `f64` product lost the low microseconds of a
/// large value (`'9223372036854' SECOND` rendered `…:53.999616` against 18.6's
/// `…:54`), and an exact-decimal product rounded a sub-microsecond tie the other way
/// from the double 18.6 actually uses (`'0.0000015' SECOND` is one microsecond
/// there, and `'0.0000015 seconds'` — the same value through the existing decoder —
/// already answered one). The delegation is exact for **every** value measured
/// against 18.6 across all six units, the fractional and negative forms, and both
/// ends of each unit's range.
///
/// It inherits that decoder's two overflow classes rather than creating a third: a
/// year count whose *product* overflows is `22008`, the assembled result's, and a field
/// past its own range is `22015` — each 18.6's code, and each what the
/// `INTERVAL '178956971 years'` spelling answers, so the two spellings answer alike.
fn interval_from_unit(literal: &str, unit: IntervalField) -> Result<PgInterval> {
    let body = literal.trim();
    PgInterval::parse(&format!("{body} {}", unit.sql_word())).map_err(|e| match e {
        // The synthesized unit word is an implementation detail of the delegation;
        // 18.6's message quotes the field the user wrote, so put it back.
        Error::IntervalFieldOverflow(_) => Error::IntervalFieldOverflow(body.to_string()),
        Error::InvalidIntervalSyntax(_) => Error::InvalidIntervalSyntax(body.to_string()),
        other => other,
    })
}

/// Zero the fields to the right of `unit`, keeping those to its left —
/// `timestamp.c`'s `AdjustIntervalForTypmod` for a single-field range.
///
/// Measured on 18.6: `'1 year 3 months' YEAR` is `1 year` (the month count rounds
/// down to a whole year), `'1 day 3 hours' DAY` is `1 day`, and `'1 day' MINUTE` is
/// still `1 day` — a lower-order mask never reaches a higher-order field.
const fn mask_interval(iv: PgInterval, unit: IntervalField) -> PgInterval {
    const MICROS_PER_HOUR: i64 = 3_600_000_000;
    const MICROS_PER_MINUTE: i64 = 60_000_000;
    match unit {
        IntervalField::Year => PgInterval::new(iv.months / 12 * 12, 0, 0),
        IntervalField::Month => PgInterval::new(iv.months, 0, 0),
        IntervalField::Day => PgInterval::new(iv.months, iv.days, 0),
        IntervalField::Hour => PgInterval::new(
            iv.months,
            iv.days,
            iv.micros / MICROS_PER_HOUR * MICROS_PER_HOUR,
        ),
        IntervalField::Minute => PgInterval::new(
            iv.months,
            iv.days,
            iv.micros / MICROS_PER_MINUTE * MICROS_PER_MINUTE,
        ),
        IntervalField::Second => iv,
    }
}

fn binop(left: Expr, op: BinaryOp, right: Expr) -> Expr {
    Expr::BinaryOp {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use crate::catalog::Schema;
    use crate::parser::parse_expr;
    use crate::planner::{BinaryOp, BuiltinFn, Expr, Literal};

    /// Parse `sql` as an expression against an empty schema.
    fn parse(sql: &str) -> Expr {
        parse_expr(sql, &Schema::default()).expect("parse should succeed")
    }

    /// Parse `sql` as an expression, keeping the error — for the spellings whose
    /// answer *is* a refusal.
    fn parse_expr_str(sql: &str) -> crate::Result<Expr> {
        parse_expr(sql, &Schema::default())
    }

    fn col(name: &str) -> Expr {
        Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    fn int(n: i64) -> Expr {
        Expr::Literal(Literal::Int(n))
    }

    fn binop(left: Expr, op: BinaryOp, right: Expr) -> Expr {
        Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    #[test]
    fn literals_and_columns() {
        assert_eq!(parse("1"), int(1));
        assert_eq!(parse("'hi'"), Expr::Literal(Literal::String("hi".into())));
        assert_eq!(parse("true"), Expr::Literal(Literal::Bool(true)));
        assert_eq!(parse("null"), Expr::Literal(Literal::Null));
        assert_eq!(parse("foo"), col("foo"));
        assert_eq!(parse("$1"), Expr::Parameter { index: 1 });
        assert_eq!(
            parse("c.id"),
            Expr::Column {
                table: Some("c".into()),
                name: "id".into(),
            }
        );
    }

    #[test]
    fn arithmetic_precedence() {
        // `1 + 2 * 3` → `1 + (2 * 3)`
        assert_eq!(
            parse("1 + 2 * 3"),
            binop(int(1), BinaryOp::Add, binop(int(2), BinaryOp::Mul, int(3))),
        );
        // left-associative: `1 - 2 - 3` → `(1 - 2) - 3`
        assert_eq!(
            parse("1 - 2 - 3"),
            binop(binop(int(1), BinaryOp::Sub, int(2)), BinaryOp::Sub, int(3)),
        );
    }

    #[test]
    fn boolean_precedence() {
        // `a OR b AND c` → `a OR (b AND c)`
        assert_eq!(
            parse("a OR b AND c"),
            binop(
                col("a"),
                BinaryOp::Or,
                binop(col("b"), BinaryOp::And, col("c")),
            ),
        );
        // `NOT a = b` → `NOT (a = b)`
        assert_eq!(
            parse("NOT a = b"),
            Expr::Not(Box::new(binop(col("a"), BinaryOp::Eq, col("b")))),
        );
        // `NOT a AND b` → `(NOT a) AND b`
        assert_eq!(
            parse("NOT a AND b"),
            binop(Expr::Not(Box::new(col("a"))), BinaryOp::And, col("b"),),
        );
    }

    /// Unary `-` over a numeric constant folds into the constant, and over anything
    /// else stays an [`Expr::Negate`] node — Postgres's `doNegate`, whose two folded
    /// operand kinds (`Integer`, `Float`) are this parser's `Int` and `Numeric`.
    ///
    /// The fold is what makes `ORDER BY -1` a **position** rather than a constant
    /// sort key; the non-folding arms are what keep `-1::int` and `-abs(1)` ordinary
    /// expressions on both engines. Each pair here was measured on 18.6 through its
    /// `ORDER BY` spelling.
    #[test]
    fn unary_minus_folds_a_numeric_constant_and_nothing_else() {
        assert_eq!(parse("-5"), int(-5));
        // Parens are structural in `c_expr` there and here, so `-(1)` folds too.
        assert_eq!(parse("-(5)"), int(-5));
        // `doNegate` over an already-negated constant folds again: `- -1` is `1`.
        assert_eq!(parse("- -5"), int(5));
        // A decimal constant is `FCONST` there / `Numeric` here — folded, and its
        // display scale survives (`-1.50` keeps scale 2).
        assert_eq!(
            parse("-1.50"),
            Expr::Literal(Literal::Numeric(
                crate::types::PgNumeric::parse("-1.50").expect("literal parses")
            )),
        );
        // Zero has no sign in `numeric`, so `-0.0` folds to the same constant `0.0`
        // does — which is why `SELECT 0.0 AS q, -0.0 AS q` no longer refuses.
        assert_eq!(parse("-0.0"), parse("0.0"));
        // `::` outranks unary minus (`TYPECAST` > `UMINUS`), so the operand is a cast
        // node, not a constant — no fold, on either engine.
        assert!(matches!(parse("-5::int"), Expr::Negate(_)));
        // A column, a function call and a parameter are none of `doNegate`'s kinds.
        assert_eq!(parse("-a"), Expr::Negate(Box::new(col("a"))));
        assert!(matches!(parse("-abs(5)"), Expr::Negate(_)));
        assert!(matches!(parse("-$1"), Expr::Negate(_)));
        // Nor is a string / boolean / NULL constant — `A_Const` yes, `Integer`/`Float`
        // no (`SELECT -'1'` is `42725` on 18.6, not the constant `-1`).
        assert!(matches!(parse("-'1'"), Expr::Negate(_)));
        assert!(matches!(parse("-true"), Expr::Negate(_)));
        assert!(matches!(parse("-NULL"), Expr::Negate(_)));
        // unary minus binds tighter than `*`: `-a * b` → `(-a) * b`
        assert_eq!(
            parse("-a * b"),
            binop(Expr::Negate(Box::new(col("a"))), BinaryOp::Mul, col("b")),
        );
    }

    #[test]
    fn modulo_and_power_precedence() {
        // `%` sits at the multiplicative tier, left-associative.
        assert_eq!(parse("7 % 3"), binop(int(7), BinaryOp::Mod, int(3)),);
        assert_eq!(
            parse("a * b % c"),
            binop(
                binop(col("a"), BinaryOp::Mul, col("b")),
                BinaryOp::Mod,
                col("c")
            ),
        );
        // `^` binds tighter than `*`: `2 * 3 ^ 2` → `2 * (3 ^ 2)`.
        assert_eq!(
            parse("2 * 3 ^ 2"),
            binop(int(2), BinaryOp::Mul, binop(int(3), BinaryOp::Pow, int(2))),
        );
        // `^` is left-associative: `2 ^ 3 ^ 2` → `(2 ^ 3) ^ 2`.
        assert_eq!(
            parse("2 ^ 3 ^ 2"),
            binop(binop(int(2), BinaryOp::Pow, int(3)), BinaryOp::Pow, int(2)),
        );
        // Unary minus binds tighter than `^`: `-2 ^ 2` → `(-2) ^ 2` (Postgres, which
        // answers `4`). The `-2` is a folded constant, not a `Negate` node, and the
        // value is the same either way.
        assert_eq!(parse("-2 ^ 2"), binop(int(-2), BinaryOp::Pow, int(2)),);
    }

    #[test]
    fn containment_and_cast() {
        // `@>` binds tighter than `=`/comparison but looser than `+`
        assert_eq!(
            parse("refs @> $1"),
            binop(
                col("refs"),
                BinaryOp::Containment,
                Expr::Parameter { index: 1 }
            ),
        );
        // `::` is tightest: `a + b::int8` → `a + (b::int8)`
        assert_eq!(
            parse("a + b::int8"),
            binop(
                col("a"),
                BinaryOp::Add,
                Expr::Cast {
                    expr: Box::new(col("b")),
                    target_type: crate::types::TypeOid::Int8,
                },
            ),
        );
    }

    #[test]
    fn jsonb_key_and_path_operators() {
        // `?`/`?|`/`?&`/`#>`/`#>>` all parse at the `BP_OTHER` band, left-assoc.
        assert_eq!(
            parse("data ? 'a'"),
            binop(col("data"), BinaryOp::JsonExists, str_lit("a")),
        );
        assert_eq!(
            parse("data ?| ARRAY['a']"),
            binop(
                col("data"),
                BinaryOp::JsonExistsAny,
                Expr::Array(vec![str_lit("a")])
            ),
        );
        assert_eq!(
            parse("data ?& ARRAY['a']"),
            binop(
                col("data"),
                BinaryOp::JsonExistsAll,
                Expr::Array(vec![str_lit("a")])
            ),
        );
        assert_eq!(
            parse("data #> '{a,b}'"),
            binop(col("data"), BinaryOp::JsonPathGet, str_lit("{a,b}")),
        );
        assert_eq!(
            parse("data #>> '{a,b}'"),
            binop(col("data"), BinaryOp::JsonPathGetText, str_lit("{a,b}")),
        );
        // `#>` binds tighter than `=` (same band as `@>`/`->`): `a #> b = c` parses
        // as `(a #> b) = c`.
        assert_eq!(
            parse("a #> b = c"),
            binop(
                binop(col("a"), BinaryOp::JsonPathGet, col("b")),
                BinaryOp::Eq,
                col("c"),
            ),
        );
    }

    #[test]
    fn comparison_operators_are_non_associative() {
        // Postgres makes `= <> < <= > >=` `%nonassoc`: chaining two at the same level
        // is a syntax error (`42601`), not a left-associative re-bind.
        let schema = Schema::default();
        for sql in [
            "1 = 1 = true",
            "1 < 2 = true",
            "a = b = c",
            "a < b < c",
            "a <> b <> c",
            "a >= b <= c",
        ] {
            let err = parse_expr(sql, &schema).unwrap_err();
            assert_eq!(err.sqlstate(), "42601", "{sql:?} should be non-associative");
        }
        // A single comparison, and comparisons split across `AND`/`OR` (a lower,
        // separate level) or `IS` (a lower level), still parse.
        assert_eq!(parse("1 < 2"), binop(int(1), BinaryOp::Lt, int(2)));
        assert_eq!(
            parse("a = b AND c = d"),
            binop(
                binop(col("a"), BinaryOp::Eq, col("b")),
                BinaryOp::And,
                binop(col("c"), BinaryOp::Eq, col("d")),
            ),
        );
        // `a = b IS NULL` stays `(a = b) IS NULL` (IS is looser than comparison).
        assert_eq!(
            parse("a = b IS NULL"),
            Expr::IsNull(Box::new(binop(col("a"), BinaryOp::Eq, col("b")))),
        );
        // A parenthesized inner comparison is fine: `a = (b = c)`.
        assert_eq!(
            parse("a = (b = c)"),
            binop(
                col("a"),
                BinaryOp::Eq,
                binop(col("b"), BinaryOp::Eq, col("c")),
            ),
        );
    }

    #[test]
    fn is_null_postfix() {
        assert_eq!(parse("a IS NULL"), Expr::IsNull(Box::new(col("a"))));
        assert_eq!(parse("a IS NOT NULL"), Expr::IsNotNull(Box::new(col("a"))));
        // `(a = b) IS NULL` — comparison binds tighter than IS
        assert_eq!(
            parse("a = b IS NULL"),
            Expr::IsNull(Box::new(binop(col("a"), BinaryOp::Eq, col("b")))),
        );
        assert_eq!(
            parse("a IS NOT DISTINCT FROM b"),
            binop(col("a"), BinaryOp::IsNotDistinctFrom, col("b")),
        );
    }

    #[test]
    fn function_calls() {
        assert_eq!(
            parse("coalesce(a, b, c)"),
            Expr::Coalesce(vec![col("a"), col("b"), col("c")]),
        );
        assert_eq!(parse("lower(name)"), Expr::Lower(Box::new(col("name"))));
        assert_eq!(parse("now()"), Expr::Builtin(BuiltinFn::Now));
        assert_eq!(
            parse("starts_with(a, 'x')"),
            binop(
                col("a"),
                BinaryOp::StartsWith,
                Expr::Literal(Literal::String("x".into()))
            ),
        );
    }

    #[test]
    fn sql_value_functions() {
        // The parenless value functions each map to their builtin. `LOCALTIMESTAMP`
        // is the non-tz timestamp form (the same instant as `CURRENT_TIMESTAMP` /
        // `now()`, typed `timestamp` rather than `timestamptz`), and
        // `CURRENT_TIMESTAMP` is its own builtin rather than `now()`'s — the same
        // instant and type, but 18.6 renders the keyword verbatim in `pg_get_expr` and
        // names the output column `current_timestamp`.
        assert_eq!(
            parse("CURRENT_TIMESTAMP"),
            Expr::Builtin(BuiltinFn::CurrentTimestamp)
        );
        assert_eq!(parse("CURRENT_DATE"), Expr::Builtin(BuiltinFn::CurrentDate));
        assert_eq!(parse("CURRENT_TIME"), Expr::Builtin(BuiltinFn::CurrentTime));
        assert_eq!(parse("LOCALTIME"), Expr::Builtin(BuiltinFn::LocalTime));
        assert_eq!(
            parse("LOCALTIMESTAMP"),
            Expr::Builtin(BuiltinFn::LocalTimestamp),
        );
        // A value function takes no argument list — an explicit precision form
        // (`LOCALTIMESTAMP(0)`) fails loud rather than silently truncating.
        assert!(parse_expr("LOCALTIMESTAMP(0)", &Schema::default()).is_err());
    }

    #[test]
    fn parens_are_structural() {
        assert_eq!(parse("(a)"), col("a"));
        // `(a + b) * c`
        assert_eq!(
            parse("(a + b) * c"),
            binop(
                binop(col("a"), BinaryOp::Add, col("b")),
                BinaryOp::Mul,
                col("c")
            ),
        );
    }

    fn str_lit(s: &str) -> Expr {
        Expr::Literal(Literal::String(s.to_string()))
    }

    #[test]
    fn in_list_and_negation() {
        assert_eq!(
            parse("a IN (1, 2, 3)"),
            Expr::InList {
                expr: Box::new(col("a")),
                list: vec![int(1), int(2), int(3)],
                negated: false,
            },
        );
        assert_eq!(
            parse("a NOT IN (1)"),
            Expr::InList {
                expr: Box::new(col("a")),
                list: vec![int(1)],
                negated: true,
            },
        );
    }

    #[test]
    fn between_desugars() {
        // `x BETWEEN 1 AND 10` → `(x >= 1) AND (x <= 10)`
        assert_eq!(
            parse("x BETWEEN 1 AND 10"),
            binop(
                binop(col("x"), BinaryOp::Ge, int(1)),
                BinaryOp::And,
                binop(col("x"), BinaryOp::Le, int(10)),
            ),
        );
        // `x NOT BETWEEN 1 AND 10` → `(x < 1) OR (x > 10)`
        assert_eq!(
            parse("x NOT BETWEEN 1 AND 10"),
            binop(
                binop(col("x"), BinaryOp::Lt, int(1)),
                BinaryOp::Or,
                binop(col("x"), BinaryOp::Gt, int(10)),
            ),
        );
    }

    #[test]
    fn like_and_array_and_quantifier() {
        assert_eq!(
            parse("name LIKE 'a%'"),
            binop(col("name"), BinaryOp::Like, str_lit("a%")),
        );
        assert_eq!(
            parse("name NOT LIKE 'a%'"),
            binop(col("name"), BinaryOp::NotLike, str_lit("a%")),
        );
        assert_eq!(parse("ARRAY[1, 2]"), Expr::Array(vec![int(1), int(2)]));
        assert_eq!(parse("ARRAY[]"), Expr::Array(vec![]));
        // `id = ANY($1)`
        assert_eq!(
            parse("id = ANY($1)"),
            Expr::AnyAll {
                left: Box::new(col("id")),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Parameter { index: 1 }),
                all: false,
                sub_select: false,
            },
        );
        assert_eq!(
            parse("id = ALL($1)"),
            Expr::AnyAll {
                left: Box::new(col("id")),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Parameter { index: 1 }),
                all: true,
                sub_select: false,
            },
        );
    }

    #[test]
    fn at_time_zone() {
        assert_eq!(
            parse("ts AT TIME ZONE 'UTC'"),
            binop(col("ts"), BinaryOp::AtTimeZone, str_lit("UTC")),
        );
    }

    #[test]
    fn at_is_usable_as_a_column_name() {
        // `AT` is UNRESERVED in Postgres. A bare `at` in a name (Pratt nud) position
        // is a column reference; the postfix `AT TIME ZONE` (led position) still
        // binds. The combined form exercises both readings of the word at once.
        assert_eq!(parse("at"), col("at"));
        assert_eq!(
            parse("at AT TIME ZONE 'UTC'"),
            binop(col("at"), BinaryOp::AtTimeZone, str_lit("UTC")),
        );
    }

    #[test]
    fn unreserved_keywords_are_bare_columns() {
        // Postgres accepts these as bare column refs (`ColId` = UNRESERVED ∪
        // COL_NAME); pglet must too.
        assert_eq!(parse("update"), col("update")); // UNRESERVED
        assert_eq!(parse("set"), col("set")); // UNRESERVED
        assert_eq!(parse("by"), col("by")); // UNRESERVED
        assert_eq!(parse("recursive"), col("recursive")); // UNRESERVED
        assert_eq!(parse("between"), col("between")); // COL_NAME
        assert_eq!(parse("values"), col("values")); // COL_NAME
    }

    #[test]
    fn type_func_name_keywords_reject_as_bare_columns() {
        // `is`/`like`/`left`/`join` are TYPE_FUNC_NAME — Postgres rejects them as
        // bare columns (`ColId` excludes them), so pglet does too (correct, not
        // over-reservation). Each is a `42601` syntax error.
        let schema = Schema::default();
        for kw in ["is", "like", "ilike", "left", "right", "join", "natural"] {
            let err = parse_expr(kw, &schema).unwrap_err();
            assert_eq!(
                err.sqlstate(),
                "42601",
                "{kw} should reject as a bare column"
            );
        }
    }

    #[test]
    fn qualified_attr_is_collabel() {
        // After `.`, the attribute is a `ColLabel`, which admits *any* keyword —
        // including TYPE_FUNC_NAME (`t.left`) and reserved (`t.select`/`c.order`)
        // words Postgres rejects as bare columns.
        let qual = |t: &str, n: &str| Expr::Column {
            table: Some(t.to_string()),
            name: n.to_string(),
        };
        assert_eq!(parse("t.left"), qual("t", "left"));
        assert_eq!(parse("t.select"), qual("t", "select"));
        assert_eq!(parse("c.order"), qual("c", "order"));
        assert_eq!(parse("c.from"), qual("c", "from"));
    }

    #[test]
    fn exists_is_column_without_paren() {
        // `exists` is COL_NAME — a bare `exists` (no `(`) is an ordinary column,
        // while `EXISTS (subquery)` still parses as the operator.
        assert_eq!(parse("exists"), col("exists"));
        assert!(matches!(
            parse("EXISTS (SELECT 1)"),
            Expr::Exists { negated: false, .. }
        ));
    }

    #[test]
    fn string_function_special_forms() {
        use crate::planner::StringFunc;
        let strfn = |func, args| Expr::StringFn { func, args };
        // SUBSTRING(x FROM n FOR m) and the comma form converge on one node.
        let expected = strfn(StringFunc::Substring, vec![col("x"), int(2), int(3)]);
        assert_eq!(parse("substring(x FROM 2 FOR 3)"), expected);
        assert_eq!(parse("substring(x, 2, 3)"), expected);
        // FOR-only defaults the start to 1; FROM-only is the two-arg form.
        assert_eq!(
            parse("substring(x FOR 2)"),
            strfn(StringFunc::Substring, vec![col("x"), int(1), int(2)]),
        );
        assert_eq!(
            parse("substring(x FROM 2)"),
            strfn(StringFunc::Substring, vec![col("x"), int(2)]),
        );
        // TRIM direction → btrim/ltrim/rtrim; `chars FROM x` puts the string first.
        assert_eq!(
            parse("trim(both 'x' from y)"),
            strfn(StringFunc::Btrim, vec![col("y"), str_lit("x")]),
        );
        assert_eq!(
            parse("trim(leading from y)"),
            strfn(StringFunc::Ltrim, vec![col("y")]),
        );
        assert_eq!(
            parse("trim('x' from y)"),
            strfn(StringFunc::Btrim, vec![col("y"), str_lit("x")]),
        );
        // POSITION(sub IN str) → StringFunc::Position with swapped [str, sub] args
        // (evals like strpos, but keeps the "position" output name).
        assert_eq!(
            parse("position('om' in name)"),
            strfn(StringFunc::Position, vec![col("name"), str_lit("om")]),
        );
    }

    #[test]
    fn type_func_name_keywords_before_a_paren_are_calls() {
        use crate::planner::StringFunc;
        use crate::types::TypeOid;
        let strfn = |func, args| Expr::StringFn { func, args };
        // `left` / `right` are TYPE_FUNC_NAME keywords: a call before `(`, in each of
        // the three spellings 18.6's grammar admits, and one node however spelled.
        let left = strfn(StringFunc::Left, vec![col("x"), int(2)]);
        assert_eq!(parse("left(x, 2)"), left);
        assert_eq!(parse("LEFT(x, 2)"), left);
        assert_eq!(parse("\"left\"(x, 2)"), left);
        assert_eq!(parse("pg_catalog.left(x, 2)"), left);
        assert_eq!(
            parse("right(x, -1)"),
            strfn(StringFunc::Right, vec![col("x"), int(-1)]),
        );
        // A string literal in an `int4` position is that type's cast; the first argument
        // keeps its literal.
        assert_eq!(
            parse("left('ab', '1')"),
            strfn(
                StringFunc::Left,
                vec![
                    Expr::Literal(Literal::String("ab".into())),
                    Expr::Cast {
                        expr: Box::new(Expr::Literal(Literal::String("1".into()))),
                        target_type: TypeOid::Int4,
                    },
                ],
            ),
        );
        // Bare, the word stays refused as a column, as `ColId` refuses it on 18.6.
        assert!(parse_expr_str("left").is_err());
        assert!(parse_expr_str("left + 1").is_err());
        // And in an operand's tail it is the operator it always was: `x LIKE y` is not
        // a call of `like`, though `like(…)` at an operand's start would be.
        assert_eq!(
            parse("x LIKE (y)"),
            binop(col("x"), BinaryOp::Like, col("y")),
        );
    }

    #[test]
    fn call_spelled_operators_resolve_unknown_literals() {
        use crate::types::TypeOid;
        let cast = |s: &str, t| Expr::Cast {
            expr: Box::new(Expr::Literal(Literal::String(s.into()))),
            target_type: t,
        };
        // `mod` / `power` / `pow` are their operators under call nodes of their own.
        assert_eq!(parse("mod(a, 2)"), binop(col("a"), BinaryOp::ModFn, int(2)));
        assert_eq!(
            parse("power(a, 2)"),
            binop(col("a"), BinaryOp::PowerFn, int(2))
        );
        assert_eq!(parse("pow(a, 2)"), binop(col("a"), BinaryOp::PowFn, int(2)));
        // A string literal beside a number literal takes the resolved type; two take
        // `float8` for `power` and are ambiguous for `mod`.
        assert_eq!(
            parse("mod(7, '3')"),
            binop(int(7), BinaryOp::ModFn, cast("3", TypeOid::Int4)),
        );
        assert_eq!(
            parse("power('2', '3')"),
            binop(
                cast("2", TypeOid::Float8),
                BinaryOp::PowerFn,
                cast("3", TypeOid::Float8)
            ),
        );
        assert!(
            matches!(
                parse("power(2.5, '2')"),
                Expr::BinaryOp { op: BinaryOp::PowerFn, right, .. }
                    if *right == cast("2", TypeOid::Numeric)
            ),
            "a string literal beside a numeric literal is numeric"
        );
        assert_eq!(
            parse_expr_str("mod('7', '3')").map_err(|e| e.sqlstate().to_string()),
            Err("42725".to_string())
        );
        assert_eq!(
            parse_expr_str("mod(NULL, NULL)").map_err(|e| e.sqlstate().to_string()),
            Err("42725".to_string())
        );
        // Beside a column the literal stays as written (no type is known yet).
        assert_eq!(
            parse("mod(a, '3')"),
            binop(
                col("a"),
                BinaryOp::ModFn,
                Expr::Literal(Literal::String("3".into()))
            ),
        );
    }

    #[test]
    fn string_function_words_are_bare_columns_without_paren() {
        // substring/trim/position are COL_NAME keywords: a bare one (no `(`) is an
        // ordinary column reference, while the paren form is the special grammar.
        assert_eq!(parse("substring"), col("substring"));
        assert_eq!(parse("trim"), col("trim"));
        assert_eq!(parse("position"), col("position"));
        // `extract` is likewise a COL_NAME keyword — bare, it is a column reference.
        assert_eq!(parse("extract"), col("extract"));
    }

    #[test]
    fn extract_special_form_and_comma_form() {
        let ext = |source, field| Expr::Extract {
            source: Box::new(source),
            field: Box::new(field),
            retnumeric: true,
        };
        // `EXTRACT(field FROM source)`: the field word becomes a text literal.
        assert_eq!(
            parse("extract(year from ts)"),
            ext(col("ts"), str_lit("year")),
        );
        // A string-constant field is also accepted in the FROM form.
        assert_eq!(
            parse("extract('dow' from ts)"),
            ext(col("ts"), str_lit("dow")),
        );
        // The plain comma form `extract(field, source)` builds the same node.
        assert_eq!(
            parse("extract('epoch', iv)"),
            ext(col("iv"), str_lit("epoch")),
        );
    }

    #[test]
    fn string_function_arity_error_is_undefined_function() {
        // A wrong argument count resolves to no function — 42883, matching Postgres.
        let schema = Schema::default();
        let err = parse_expr("upper('a', 'b')", &schema).unwrap_err();
        assert_eq!(err.sqlstate(), "42883");
    }

    #[test]
    fn unsupported_fails_loud() {
        let schema = Schema::default();
        // unknown function → 0A000 (not implemented)
        let err = parse_expr("frobnicate(x)", &schema).unwrap_err();
        assert_eq!(err.sqlstate(), "0A000");
        // unsupported type in a cast → 42704 (`point` is still unimplemented;
        // `time`/`numeric` graduated to real types and now parse)
        let err = parse_expr("x::point", &schema).unwrap_err();
        assert_eq!(err.sqlstate(), "42704");
        // leftover input → 42601
        let err = parse_expr("a b", &schema).unwrap_err();
        assert_eq!(err.sqlstate(), "42601");
    }

    #[test]
    fn cast_routes_through_the_shared_type_name_reader() {
        use crate::types::TypeOid;
        // A length/precision-bearing target parses to `CastTypmod`; a bare one to
        // `Cast`. Read the target type out of either.
        let cast_to = |sql: &str| match parse(sql) {
            Expr::Cast { target_type, .. } => target_type,
            Expr::CastTypmod { type_oid, .. } => type_oid,
            other => panic!("expected a cast, got {other:?}"),
        };
        // Multi-word and length-bearing spellings now parse (the old
        // single-identifier reader left `precision` / `(255)` behind).
        assert_eq!(cast_to("x::int8"), TypeOid::Int8);
        assert_eq!(cast_to("CAST(x AS double precision)"), TypeOid::Float8);
        assert_eq!(cast_to("x::varchar(255)"), TypeOid::Varchar);
        // `time` / `time without time zone` graduated to a real column type.
        assert_eq!(cast_to("x::time"), TypeOid::Time);
        assert_eq!(cast_to("CAST(x AS time without time zone)"), TypeOid::Time);
        assert_eq!(
            cast_to("CAST(x AS timestamp with time zone)"),
            TypeOid::Timestamptz,
        );
        assert_eq!(cast_to("x::text[]"), TypeOid::TextArray);
    }

    #[test]
    fn typmod_cast_builds_cast_typmod() {
        use crate::planner::Typmod;
        use crate::types::TypeOid;
        // A length/precision-bearing cast target builds `CastTypmod` (which enforces
        // the modifier at eval time), keeping the captured typmod; a bare target
        // stays a plain `Cast`.
        assert_eq!(
            parse("x::varchar(3)"),
            Expr::CastTypmod {
                expr: Box::new(col("x")),
                type_oid: TypeOid::Varchar,
                typmod: Typmod::Length(3),
            }
        );
        assert_eq!(
            parse("CAST(x AS char(5))"),
            Expr::CastTypmod {
                expr: Box::new(col("x")),
                type_oid: TypeOid::Varchar,
                typmod: Typmod::Length(5),
            }
        );
        assert_eq!(
            parse("x::numeric(4,1)"),
            Expr::CastTypmod {
                expr: Box::new(col("x")),
                type_oid: TypeOid::Numeric,
                typmod: Typmod::Numeric {
                    precision: 4,
                    scale: 1,
                },
            }
        );
        // A bare `varchar` / `numeric` (no modifier) stays a plain `Cast`.
        assert!(matches!(parse("x::varchar"), Expr::Cast { .. }));
        assert!(matches!(parse("x::numeric"), Expr::Cast { .. }));
    }

    /// The four types that carry a **sub-second precision** build a `CastTypmod` too,
    /// at every spelling of each — including the ones where 18.6 writes the `(p)` in
    /// the middle of the name.
    ///
    /// `p = 6` is a *captured* modifier whose rounding is a no-op, and `p > 6` is
    /// clamped to `6` (18.6 warns and records `6`), so both build a node where a bare
    /// `timestamp` builds a plain `Cast`.
    #[test]
    fn a_temporal_precision_cast_builds_cast_typmod() {
        use crate::planner::Typmod;
        use crate::types::TypeOid;
        let typmod = |src| match parse(src) {
            Expr::CastTypmod {
                type_oid, typmod, ..
            } => (type_oid, typmod),
            other => panic!("expected a CastTypmod for `{src}`, got {other:?}"),
        };
        assert_eq!(
            typmod("x::timestamp(3)"),
            (TypeOid::Timestamp, Typmod::Precision(3))
        );
        assert_eq!(
            typmod("x::timestamp(3) without time zone"),
            (TypeOid::Timestamp, Typmod::Precision(3))
        );
        assert_eq!(
            typmod("x::timestamptz(3)"),
            (TypeOid::Timestamptz, Typmod::Precision(3))
        );
        assert_eq!(
            typmod("x::timestamp(3) with time zone"),
            (TypeOid::Timestamptz, Typmod::Precision(3))
        );
        assert_eq!(typmod("x::time(3)"), (TypeOid::Time, Typmod::Precision(3)));
        assert_eq!(
            typmod("CAST(x AS interval(3))"),
            (TypeOid::Interval, Typmod::Precision(3))
        );
        assert_eq!(typmod("x::time(0)"), (TypeOid::Time, Typmod::Precision(0)));
        assert_eq!(
            typmod("x::timestamp(6)"),
            (TypeOid::Timestamp, Typmod::Precision(6))
        );
        // Clamped, not refused and not dropped — 18.6 records `atttypmod = 6`.
        assert_eq!(
            typmod("x::timestamp(9)"),
            (TypeOid::Timestamp, Typmod::Precision(6))
        );
        assert_eq!(
            typmod("x::timestamp(2147483647)"),
            (TypeOid::Timestamp, Typmod::Precision(6))
        );
        // A bare spelling carries no modifier at all.
        assert!(matches!(parse("x::timestamp"), Expr::Cast { .. }));
        assert!(matches!(parse("x::interval"), Expr::Cast { .. }));
    }

    /// **The canonical spelling carries the precision LAST, and that is a correctness
    /// decision.** `executor::type_resolution`'s `decompose_type_name` cuts on the
    /// first `(`, so a mid-name `time(3) with time zone` would decompose to base
    /// `time` and silently build a `time(3)` out of a type pglet does not support.
    ///
    /// Both directions: the unsupported tz-time spellings stay
    /// `42704 type "time with time zone" is not supported` **with** a precision, as
    /// they already are without one; and the supported ones still resolve.
    #[test]
    fn a_tz_time_precision_is_still_an_unsupported_type() {
        for src in [
            "x::timetz(3)",
            "x::time(3) with time zone",
            "x::timetz",
            "x::time with time zone",
        ] {
            let err = parse_expr_str(src).expect_err(src);
            assert_eq!(err.sqlstate(), "42704", "{src}");
            assert_eq!(
                err.to_string(),
                "type \"time with time zone\" is not supported",
                "{src}"
            );
        }
        // The polarity control: the same window over a supported type resolves.
        assert!(parse_expr_str("x::time(3)").is_ok());
        assert!(parse_expr_str("x::time(3) without time zone").is_ok());
    }

    /// A typed constant is the string literal read as that type, and lowers to the node
    /// `'…'::type` lowers to — so nothing downstream (declared type, deparse, typmod
    /// enforcement) has a second shape to learn.
    ///
    /// **One type name resolves differently at the two positions**, which is
    /// `a_bare_char_constant_is_not_the_cast` below; every spelling here is one the two
    /// share.
    #[test]
    fn a_typed_constant_lowers_to_the_cast_it_is() {
        assert_eq!(parse("DATE '2024-06-15'"), parse("'2024-06-15'::date"));
        assert_eq!(parse("text 'x'"), parse("'x'::text"));
        assert_eq!(parse("INT4 '7'"), parse("'7'::int4"));
        // Multi-word and modifier-bearing type names take the speculative path.
        assert_eq!(
            parse("DOUBLE PRECISION '1.5'"),
            parse("'1.5'::double precision")
        );
        assert_eq!(
            parse("CHARACTER VARYING 'ab'"),
            parse("'ab'::character varying")
        );
        assert_eq!(
            parse("TIMESTAMP WITH TIME ZONE '2024-06-15'"),
            parse("'2024-06-15'::timestamp with time zone")
        );
        assert_eq!(parse("NUMERIC(5,2) '1.5'"), parse("'1.5'::numeric(5,2)"));
        assert_eq!(parse("VARCHAR(3) 'abcd'"), parse("'abcd'::varchar(3)"));
        // `FLOAT(n)`'s modifier selects the type, so the constant it builds is the
        // *selected* type's — indistinguishable from the spelling that names it.
        assert_eq!(parse("FLOAT(1) '0.5'"), parse("'0.5'::real"));
        assert_eq!(parse("FLOAT(24) '0.5'"), parse("'0.5'::real"));
        assert_eq!(parse("FLOAT(25) '0.5'"), parse("'0.5'::double precision"));
        assert_eq!(parse("FLOAT '0.5'"), parse("'0.5'::double precision"));
    }

    /// `gram.y` has a separate `ConstCharacter` so that `CHAR 'hi'` does not take the
    /// `bpchar(1)` a `char` column and an `'hi'::char` both take, and pglet splits the
    /// resolution the same way (`executor::resolve_constant_type` against
    /// `executor::resolve_column_type`). A **parenless** `char` — under either of its
    /// two spellings — is the one spelling `resolve_type_name`'s position check
    /// reaches, and it builds a bare `Cast` here against a length-1 `CastTypmod` there.
    ///
    /// Both sides of each assertion are pglet's, so this pins the node shapes and not
    /// the values; `tests/fixtures/types/typed_constant_bare_char_*` are the
    /// differential half, and the `EDGES.md` row 95 gap (`char` is `varchar` here) is
    /// why the equal-to spellings below name `varchar`.
    ///
    /// The **refused** lengths at the end are the exception to "both sides are pglet's":
    /// a refusal has no node, so they assert a SQLSTATE, and that one is 18.6's too
    /// (the `types/refused_typmod_*` fixtures carry the differential half).
    #[test]
    fn a_bare_char_constant_is_not_the_cast() {
        // The split itself.
        assert_ne!(parse("char 'ab'"), parse("'ab'::char"));
        assert_eq!(parse("character 'ab'"), parse("char 'ab'"));
        // The constant takes no length at all…
        assert_eq!(parse("char 'ab'"), parse("'ab'::varchar"));
        // …while the cast, and a **spelled** modifier at the constant position, take
        // the length-1 node.
        assert_eq!(parse("char(1) 'ab'"), parse("'ab'::char"));
        assert_eq!(parse("CAST('ab' AS character)"), parse("'ab'::char"));
        assert_eq!(parse("char(3) 'ab'"), parse("'ab'::varchar(3)"));
        // A spelled length the resolver **refuses** is a spelled modifier all the
        // same, so it takes the default at neither position — and there is no node to
        // compare, the refusal being the statement's answer at both. Both ends of the
        // window, so neither side can be the parenless spelling's `char(1)` or an
        // unconstrained `varchar`.
        for src in [
            "'ab'::char(0)",
            "char(0) 'ab'",
            "'ab'::char(20000000)",
            "char(20000000) 'ab'",
        ] {
            assert_eq!(
                parse_expr_str(src).unwrap_err().sqlstate(),
                "22023",
                "`{src}`"
            );
        }
        // The other branch of the same grammar arm spells no default at either
        // position, so its constant and cast still agree.
        assert_eq!(parse("character varying 'ab'"), parse("'ab'::varchar"));
        assert_eq!(parse("varchar(3) 'abcd'"), parse("'abcd'::varchar(3)"));
    }

    /// `FLOAT(n)` selects `float4` for 1–24 and `float8` for 25–53, and refuses
    /// `22023` at both ends — 18.6's `opt_float`, reproduced at **parse** time because
    /// that is where 18.6 raises it.
    ///
    /// The boundaries are the point: a fixture asserts one statement, and every
    /// off-by-one here (25 selecting `float4`, 53 refusing, 54 selecting) is a silent
    /// wrong value rather than an error.
    #[test]
    fn a_float_precision_selects_the_type_and_refuses_outside_the_window() {
        use crate::types::TypeOid;
        let oid = |src: &str| match parse(src) {
            Expr::Cast { target_type, .. } => target_type,
            other => panic!("expected a Cast for `{src}`, got {other:?}"),
        };
        for n in [1, 2, 23, 24] {
            assert_eq!(
                oid(&format!("x::float({n})")),
                TypeOid::Float4,
                "float({n})"
            );
        }
        for n in [25, 26, 52, 53] {
            assert_eq!(
                oid(&format!("x::float({n})")),
                TypeOid::Float8,
                "float({n})"
            );
        }
        // The `/*EMPTY*/` production is `float8` too, and takes `double precision`'s
        // canonical spelling rather than one of its own.
        assert_eq!(oid("x::float"), TypeOid::Float8);
        for (src, message) in [
            (
                "x::float(0)",
                "precision for type float must be at least 1 bit",
            ),
            (
                "x::float(54)",
                "precision for type float must be less than 54 bits",
            ),
            (
                "x::float(2147483647)",
                "precision for type float must be less than 54 bits",
            ),
        ] {
            let err = parse_expr_str(src).expect_err(src);
            assert_eq!(err.sqlstate(), "22023", "{src}");
            assert_eq!(err.to_string(), message, "{src}");
            // `P` points at the number, as `parser_errposition(@2)` does: `x::float(`
            // is nine characters, so the digit is the tenth.
            assert_eq!(err.position(), Some(10), "{src}");
        }
    }

    /// **A character length's window is one `Iconst` too**, which is what keeps the
    /// length refusal from firing under the wrong SQLSTATE.
    ///
    /// `CharacterWithLength: character '(' Iconst ')'`, so at the `character` keyword's
    /// spellings a fractional or
    /// out-of-`int32` length is a **grammar** error on 18.6 (`42601 syntax error at or
    /// near "3.5"`, measured) and never reaches `anychar_typmodin`'s `22023` — which it
    /// would have done here, answering `length for type char must be at least 1` about a
    /// number nobody could have written. (The `bpchar` spelling takes `GenericType`'s
    /// `expr_list` instead and does reach the reader, for the component coercion rather
    /// than that `22023` — `22P02` at the fraction and `22003` at the out-of-`int32`
    /// length, both measured; `EDGES.md` row 124's residual.)
    /// Reading the length through `expect_number_text`
    /// instead of `opt_precision` accepted every spelling in the first loop below and
    /// then silently dropped the modifier.
    ///
    /// The two accepted spellings are the reason the reader passes the **text** on and
    /// re-reads it later: the lexer hands `0x3` a decoded payload and keeps `03`
    /// verbatim, and both have to arrive at the resolver as the length `3`.
    ///
    /// That decode also decides what the *refusal* names, and it is the payload rather
    /// than the source slice: `'ab'::char(0xFFFFFFFF)` says `found "4294967295"` here
    /// against 18.6's `syntax error at or near "0xFFFFFFFF"`. Same SQLSTATE, and no
    /// fixture carries `@expect-message` at this window, so nothing gates the text —
    /// recorded rather than closed (`EDGES.md` row 124).
    #[test]
    fn a_character_length_is_one_iconst() {
        for src in [
            "'ab'::char(3.5)",
            "'ab'::char(2147483648)",
            "'ab'::char(99999999999999999999)",
            "'ab'::varchar(3.5)",
            "'ab'::varchar(2147483648)",
        ] {
            let err = parse_expr_str(src).expect_err(src);
            assert_eq!(err.sqlstate(), "42601", "{src}");
        }
        // …and the spellings an `Iconst` does admit reach the resolver as `3`.
        for src in ["'abcd'::char(3)", "'abcd'::char(0x3)", "'abcd'::char(03)"] {
            assert_eq!(
                parse_expr_str(src).expect(src),
                parse("'abcd'::varchar(3)"),
                "{src}"
            );
        }
    }

    /// `float (` is a committed window and bare `float` is not — the split
    /// `commits_on_open_paren` draws, from both sides.
    ///
    /// Committing is what keeps the `22023` from being swallowed by the speculative
    /// restore and demoted to the function-call path's
    /// `0A000 function not supported: float`; **not** committing on the bare word is
    /// what keeps `SELECT float` a column reference, which it is on 18.6 too.
    #[test]
    fn a_float_open_paren_commits_where_the_bare_word_restores() {
        for src in ["float(0)", "float(0) FROM t", "float(54)"] {
            let err = parse_expr_str(src).expect_err(src);
            assert_eq!(err.sqlstate(), "22023", "{src}");
        }
        // In range, with no string literal after it: the window is still committed, so
        // this is the type name's own syntax error rather than a function call.
        for src in ["float(1)", "float(1) + 1", "float(1.5)", "float(1,2)"] {
            let err = parse_expr_str(src).expect_err(src);
            assert_eq!(err.sqlstate(), "42601", "{src}");
        }
        // The control: no `(`, so the speculation restores and the word is a column.
        assert_eq!(parse("float"), col("float"));
        assert_eq!(
            parse("float + 1"),
            binop(col("float"), BinaryOp::Add, int(1))
        );
        // The other control, against **widening** the set: these five lead words are
        // not `COL_NAME_KEYWORD`s on 18.6, so `<word> (` there is an ordinary function
        // call (`SELECT bpchar(3)` is `3`, the other four
        // `42883 function … does not exist`). Committing them would replace a
        // function-call answer with a type-name one, which is the direction 18.6 does
        // not go. The lead words that *are* `COL_NAME_KEYWORD`s are deliberately
        // absent: they commit on 18.6 and not here, which is a divergence to close
        // rather than a state to lock.
        for word in ["bpchar", "varbit", "timestamptz", "timetz", "double"] {
            let err = parse_expr_str(&format!("{word}(3)")).expect_err(word);
            assert_eq!(err.sqlstate(), "0A000", "{word}(3) is a function call here");
        }
    }

    /// The speculative parse must put the cursor **back**: every one of the
    /// multi-word lead words is a legal bare column name on 18.6 and here, so a lead
    /// word not followed by a type-name-then-string still reads as a column.
    #[test]
    fn a_type_lead_word_not_followed_by_a_string_is_still_a_column() {
        for word in [
            "double",
            "character",
            "numeric",
            "decimal",
            "varchar",
            "char",
            "float",
            "bit",
            "time",
            "timestamp",
            "interval",
            "date",
        ] {
            assert_eq!(parse(word), col(word), "bare `{word}` should be a column");
            assert_eq!(
                parse(&format!("{word} + 1")),
                binop(col(word), BinaryOp::Add, int(1)),
                "`{word} + 1` should be arithmetic on a column",
            );
        }
        // A dotted reference, where the speculative type-name parse reads the lead
        // word and then finds a `.`.
        assert_eq!(
            parse("t.date"),
            Expr::Column {
                table: Some("t".to_string()),
                name: "date".to_string(),
            }
        );
    }

    /// `INTERVAL '…'` is unchanged by the typed-constant arm: it keeps its own
    /// production (and its `PgInterval` value), and the qualifier is applied.
    #[test]
    fn an_interval_literal_keeps_its_own_production() {
        use crate::types::PgInterval;
        let interval = |e: &Expr| match e {
            Expr::Bound(crate::types::Value::Interval(iv)) => *iv,
            other => panic!("expected an interval constant, got {other:?}"),
        };
        assert_eq!(
            interval(&parse("INTERVAL '1 day'")),
            PgInterval::new(0, 1, 0)
        );
        // The bare number takes the qualifier's unit — 90 *minutes*, not 90 seconds.
        assert_eq!(
            interval(&parse("INTERVAL '90' MINUTE")),
            PgInterval::new(0, 0, 5_400_000_000)
        );
        assert_eq!(
            interval(&parse("INTERVAL '1' DAY")),
            PgInterval::new(0, 1, 0)
        );
        assert_eq!(
            interval(&parse("INTERVAL '1' YEAR")),
            PgInterval::new(12, 0, 0)
        );
        // A units-bearing literal is masked, not re-read.
        assert_eq!(
            interval(&parse("INTERVAL '1 day 3 hours' DAY")),
            PgInterval::new(0, 1, 0)
        );
        assert_eq!(
            interval(&parse("INTERVAL '1 year 3 months' YEAR")),
            PgInterval::new(12, 0, 0)
        );
        // A MINUTE mask never reaches a higher-order field.
        assert_eq!(
            interval(&parse("INTERVAL '1 day' MINUTE")),
            PgInterval::new(0, 1, 0)
        );
        // `WEEK` is not in `opt_interval`, so it is not consumed here (18.6 reads it
        // as an output alias and answers `00:00:01`).
        assert_eq!(
            interval(&parse("INTERVAL '1'")),
            PgInterval::new(0, 0, 1_000_000)
        );
    }

    /// `IS` binds tighter than `NOT`/`AND` and looser than a comparison — measured
    /// on 18.6, where `NOT true IS TRUE` is `f` and `1 = 1 IS TRUE` is `t`.
    #[test]
    fn the_boolean_test_binds_between_comparison_and_not() {
        use crate::planner::BoolTest;
        let test = |expr, t| Expr::BooleanTest {
            expr: Box::new(expr),
            test: t,
        };
        assert_eq!(
            parse("a = b IS TRUE"),
            test(binop(col("a"), BinaryOp::Eq, col("b")), BoolTest::True),
        );
        assert_eq!(
            parse("NOT a IS TRUE"),
            Expr::Not(Box::new(test(col("a"), BoolTest::True))),
        );
        assert_eq!(parse("a IS NOT FALSE"), test(col("a"), BoolTest::NotFalse),);
        assert_eq!(parse("a IS UNKNOWN"), test(col("a"), BoolTest::Unknown),);
        assert_eq!(
            parse("a IS NOT UNKNOWN"),
            test(col("a"), BoolTest::NotUnknown),
        );
        // `UNKNOWN` is not a keyword, so it stays a usable column name.
        assert_eq!(parse("unknown"), col("unknown"));
    }
}
