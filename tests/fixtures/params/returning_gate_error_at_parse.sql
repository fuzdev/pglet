-- A bound parameter and an ill-typed gated clause in the same statement: the
-- statement must report its **type** error, not a protocol violation.
--
-- Postgres resolves names, types, and operators during parse analysis, so it raises
-- the `RETURNING` operator gate's `42883` at **Parse** — before the client has bound
-- anything. pglet's extended-protocol Parse handler used to swallow every plan
-- failure (`parse_single(…).ok().flatten()`), which is why this shape needed a lock:
-- with no plan cached, `infer_param_types` never ran, the prepared statement
-- advertised **zero** parameters, and Bind rejected the client's one value with
-- `08P01` "bind message supplies 1 parameters, but … requires 0". The SQLSTATE was
-- not merely late but the wrong class — a protocol violation reported for a
-- well-formed protocol exchange.
--
-- This is the first fixture to pair `@param` with `@expect-error`, and that gap is
-- what let the shape through: `@param` fixtures all expected success, and every
-- error fixture used literal SQL, so no fixture ever bound a parameter into a
-- statement that fails to plan. Both directions matter — `RETURNING` only joined
-- the gated clauses last, and before it did the engine already reported
-- `42883` there, so the protocol-shaped answer was a regression in error class
-- on the spine's real binding path.
--
-- The `@param` leg is what makes it a lock: the same SQL with a literal in place of
-- `$1` never reaches Bind, so it cannot see the count check at all.
-- @setup
CREATE TABLE pgate_r (i4 INT4, tx TEXT);
INSERT INTO pgate_r VALUES (7, 'a');
-- @param $1 text z
-- @test
-- @expect-error: the RETURNING operand gate's 42883, not Bind's 08P01 parameter-count violation
UPDATE pgate_r SET tx = $1 RETURNING i4 LIKE 'x%';
