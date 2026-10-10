-- The `ON CONFLICT … DO UPDATE SET` right-hand side was coerced to the target column's
-- type and nothing else — the one DML assignment position the plan-time walk never
-- reached. `coerce_to_type` says what the value must *become*; it never says whether the
-- expression is well-typed on its own, so this ran and wrote `'10'` into `tx` where
-- Postgres rejects the statement.
-- @setup
CREATE TABLE ocsc (id INT PRIMARY KEY, i4 INT4, tx TEXT);
INSERT INTO ocsc VALUES (1, 10, 'a');
-- @profile-lock
-- @expect-error: COALESCE branches have no common type in the DO UPDATE SET (42804)
-- @test
INSERT INTO ocsc VALUES (1, 10, 'a') ON CONFLICT (id) DO UPDATE SET tx = COALESCE(ocsc.i4, ocsc.tx);
