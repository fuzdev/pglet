-- An **explicit** cast on the parameter types it, and the gate then refuses the
-- statement — which is 18.6's answer too. What this pins is that the cast's type
-- reaches the gate rather than the boolean context's inference swallowing it; the
-- *priority* itself is invisible here, the refusal landing at `Parse` before any
-- `ParameterDescription` is read (`boolean_context_verdicts.rs` reaches it).
-- @setup
CREATE TABLE bcp3 (a INT);
INSERT INTO bcp3 VALUES (1);
-- @param $1 int4 1
-- @expect-error: argument of WHERE must be type boolean, not type integer
-- @test
SELECT count(*) FROM bcp3 WHERE $1::int;
