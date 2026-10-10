-- A typmod'd cast is concrete at its target type for branch unification, so
-- `COALESCE(x::numeric(4,1), 0)` unifies the integer `0` branch to numeric (rather
-- than treating the cast as an unknown type). Guards the declared-vs-returned type
-- agreement the branch-type helpers (`concrete_branch_type`) provide.
-- @setup
CREATE TABLE ct (id int, x numeric);
INSERT INTO ct VALUES (1, 12.345), (2, NULL);
-- @test
SELECT id, COALESCE(x::numeric(4,1), 0) AS r FROM ct ORDER BY id;
