-- Two accepted key spellings over a set op. A parenthesized column is not a wrapper at
-- all here (parens are structural in the expression parser), so `(a)` is the same
-- `Expr::Column` a bare `a` is and is judged whole — measured on 18.6, `(bogus)` is
-- `42703` there, so that is right. A cast is a computed key whose `Column` child the
-- leaf walk judges — `bogus::int` is `42703` on both engines — and `a::int` is accepted
-- on both because its leaf binds here and the no-op cast folds away there (`coerce_type`
-- returns the input unchanged, so 18.6's key is a plain Var; `a::text` does not fold and
-- is `0A000` there while pglet's leniency sorts it).
--
-- Both keys in one statement, since either one alone would pass a gate that got the other
-- wrong.
-- @setup
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);
INSERT INTO sopa VALUES (3), (1);
INSERT INTO sopb VALUES (2);

-- @test
SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY (a), a::int;
