//! Minimal `DO $$ … $$` anonymous-block parsing.
//!
//! pglet supports the **idempotent-`CREATE TYPE`** PL/pgSQL idiom the fuz_app cell
//! migration uses — `DO $$ BEGIN <stmts>; [EXCEPTION WHEN <cond> THEN <handler>;]
//! END $$` — and nothing more. The body is a fresh sub-parse of the dollar-quoted
//! text (the lexer already decodes the `$$…$$` body to its inner SQL); the
//! statements lower to ordinary [`Plan`]s and the EXCEPTION condition names resolve
//! to SQLSTATE codes at parse time. Anything beyond this minimal shape (a `DECLARE`
//! section, variables, control flow, an unrecognized condition) fails loud — full
//! PL/pgSQL is deliberately out of scope, to keep the size-budget floor intact.

use crate::lexer::{Keyword, TokenKind};
use crate::planner::{DoExceptionHandler, Plan};
use crate::{Error, Result};

use super::Parser;

impl Parser<'_> {
    /// Parse `DO [LANGUAGE <lang>] $$ … $$ [LANGUAGE <lang>]` into a
    /// [`Plan::DoBlock`].
    pub(super) fn parse_do(&mut self) -> Result<Plan> {
        self.enter()?;
        let result = self.parse_do_inner();
        self.leave();
        result
    }

    fn parse_do_inner(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Do)?;
        // Postgres allows `LANGUAGE <lang>` on either side of the code body.
        self.eat_language_clause()?;
        // The code body — a (dollar-)quoted string literal whose decoded text is
        // the PL/pgSQL source.
        let body_tok = self.ts.advance()?;
        if !matches!(body_tok.kind, TokenKind::String) {
            return Err(self.ts.expected("a quoted DO block body"));
        }
        let body_src = self.ts.text(&body_tok).to_string();
        self.eat_language_clause()?;

        // Parse the body with a fresh sub-parser over its text, against the same
        // live schema (so an inner `CREATE TYPE` plans against the real catalog).
        let mut sub = Parser::new(&body_src, self.schema)?;
        let plan = sub.parse_plpgsql_block();
        let plan = sub.settle(plan)?;
        if !sub.ts.at_eof() {
            sub.settle::<()>(Err(sub.ts.expected("end of DO block body")))?;
        }
        // A sub-parser's **deferred** state does not flow out with its plan, and a
        // deferred refusal that nobody adopts is an accept. `parse()` fires
        // `lock_wait_policy_refusal` off the *outer* parser only, so a `NOWAIT` /
        // `SKIP LOCKED` inside the body recorded itself here and was dropped with
        // `sub`: `DO $$ BEGIN UPDATE m SET a=1 WHERE a IN (SELECT a FROM t FOR UPDATE
        // NOWAIT); END $$` reported `DO` and ran, where every other route — the
        // reference site, a CTE body, a scalar subquery, `INSERT … SELECT`, a `DEFAULT`
        // subquery, the extended-protocol `Parse` — is `0A000`.
        self.adopt_deferred_refusals(&mut sub);
        Ok(plan)
    }

    /// Consume an optional `LANGUAGE <lang>` clause (`LANGUAGE` and the language name
    /// are both non-reserved identifiers). pglet accepts any language name and
    /// ignores it — the body shape is what it actually parses.
    fn eat_language_clause(&mut self) -> Result<()> {
        if self.eat_ident("language")? {
            let tok = self.ts.advance()?;
            if !matches!(
                tok.kind,
                TokenKind::Identifier | TokenKind::QuotedIdentifier
            ) {
                return Err(self.ts.expected("a language name after LANGUAGE"));
            }
        }
        Ok(())
    }

    /// Parse the PL/pgSQL block body: `BEGIN <stmts> [EXCEPTION …] END [;]`.
    fn parse_plpgsql_block(&mut self) -> Result<Plan> {
        if self.at_ident("declare") {
            return Err(Error::NotImplemented(
                "DECLARE in a DO block is not supported".into(),
            ));
        }
        self.ts.expect_kw(Keyword::Begin)?;
        let body = self.parse_plpgsql_statements(true)?;
        let exception = if self.at_ident("exception") {
            self.ts.bump()?;
            Some(self.parse_plpgsql_exception()?)
        } else {
            None
        };
        self.ts.expect_kw(Keyword::End)?;
        self.ts.eat(&TokenKind::Semicolon)?;
        Ok(Plan::DoBlock { body, exception })
    }

    /// Parse PL/pgSQL statements until `END` (or, when `stop_at_exception`, the
    /// contextual `EXCEPTION`). A bare `NULL` is the PL/pgSQL no-op — it lowers to no
    /// plan; every other statement lowers through the ordinary
    /// [`Parser::parse_statement`]. Each statement is `;`-terminated (Postgres
    /// requires it, the last one before `END` included).
    fn parse_plpgsql_statements(&mut self, stop_at_exception: bool) -> Result<Vec<Plan>> {
        let mut stmts = Vec::new();
        loop {
            if self.ts.at_kw(Keyword::End) || (stop_at_exception && self.at_ident("exception")) {
                break;
            }
            if self.ts.at_kw(Keyword::Null) {
                self.ts.bump()?; // the no-op statement
            } else {
                let stmt = self.parse_statement()?;
                reject_transaction_control(&stmt)?;
                stmts.push(stmt);
            }
            self.ts.expect(&TokenKind::Semicolon)?;
        }
        Ok(stmts)
    }

    /// Parse `WHEN <condition> [OR <condition>]* THEN <handler-stmts>` after the
    /// `EXCEPTION` keyword.
    fn parse_plpgsql_exception(&mut self) -> Result<DoExceptionHandler> {
        self.ts.expect_kw(Keyword::When)?;
        let mut sqlstates = Vec::new();
        let mut catch_all = false;
        loop {
            let cond = self.expect_condition_name()?;
            if cond == "others" {
                catch_all = true;
            } else {
                sqlstates.push(condition_to_sqlstate(&cond)?.to_string());
            }
            if !self.ts.eat_kw(Keyword::Or)? {
                break;
            }
        }
        self.ts.expect_kw(Keyword::Then)?;
        let handler = self.parse_plpgsql_statements(false)?;
        Ok(DoExceptionHandler {
            sqlstates,
            catch_all,
            handler,
        })
    }

    /// Read a PL/pgSQL exception condition name (`duplicate_object`, `others`, …) —
    /// a bare identifier, downcased like any other unquoted name.
    fn expect_condition_name(&mut self) -> Result<String> {
        let tok = self.ts.advance()?;
        if matches!(tok.kind, TokenKind::Identifier) {
            Ok(self.ts.name(&tok))
        } else {
            Err(self.ts.expected("an exception condition name"))
        }
    }
}

/// Refuse a transaction-control statement in a `DO` body, where it would otherwise
/// be a **silent no-op**.
///
/// The lifecycle a transaction-control plan asks for is applied by the *host*, which
/// reads the signal off the statement's own `QueryResult` — and a `DO` block's inner
/// results never reach it. So a `SAVEPOINT` inside a body would parse, execute to a
/// command tag, and change nothing, with the surrounding writes standing as though it
/// had worked.
///
/// **This gate is not decoration: without it, adding savepoints to the parser
/// *created* that hole.** Before they existed, `SAVEPOINT` and `RELEASE` were `0A000`
/// from the statement dispatcher and `ROLLBACK TO` was `42601` — which is what 18.6
/// answers for those three inside a `DO` body, so the divergence was closed by
/// accident. Teaching the parser those statements silently opened it. The general
/// shape is worth carrying: a rule that claims a class of input has to be checked
/// against what those inputs did *before* it, because some of them were already
/// right for a reason no fixture recorded.
///
/// Two deliberate divergences, both loud on both engines:
///
/// - `ROLLBACK TO SAVEPOINT` is `42601` on 18.6 (PL/pgSQL's own grammar has no `TO`
///   tail) and `0A000` here — one gate for the whole family rather than reproducing
///   which of them PL/pgSQL's parser happens to reach first.
/// - `COMMIT` and `ROLLBACK` are **accepted** inside a `DO` body on 18.6 (PL/pgSQL
///   has supported them since 11) and refused here. That is an over-rejection
///   replacing a silent wrong, not a regression: pglet's inner `COMMIT` reported
///   success and committed nothing, for the same reason as above. Refusing the whole
///   family also keeps the gate an enumeration of *plan kinds* rather than a list of
///   the ones that happen to be wrong today.
fn reject_transaction_control(plan: &Plan) -> Result<()> {
    if matches!(
        plan,
        Plan::BeginTransaction { .. }
            | Plan::CommitTransaction
            | Plan::RollbackTransaction
            | Plan::Savepoint { .. }
            | Plan::RollbackToSavepoint { .. }
            | Plan::ReleaseSavepoint { .. }
    ) {
        return Err(Error::NotImplemented(
            "unsupported transaction command in PL/pgSQL".into(),
        ));
    }
    Ok(())
}

/// Map a PL/pgSQL exception **condition name** to its SQLSTATE code (the subset
/// pglet's idempotent-guard idioms need). An unrecognized name fails loud rather
/// than silently never matching — better a clear "unsupported" error than a
/// swallow that catches nothing. `OTHERS` (the catch-all) is handled by the caller.
fn condition_to_sqlstate(name: &str) -> Result<&'static str> {
    // `name` arrives already downcased from `expect_condition_name` (an unquoted
    // identifier), so the comparison is exact.
    Ok(match name {
        "duplicate_object" => "42710",
        "duplicate_table" => "42P07",
        "duplicate_column" => "42701",
        "duplicate_database" => "42P04",
        "duplicate_schema" => "42P06",
        "duplicate_function" => "42723",
        "unique_violation" => "23505",
        _ => {
            return Err(Error::NotImplemented(format!(
                "DO block EXCEPTION condition `{name}` is not supported"
            )));
        }
    })
}
