-- A **bound** `$1` as the boolean test's operand. The parameter is cast, and that is
-- load-bearing rather than decoration: pglet's parameter walk types a `$N` from a
-- cast and not from a boolean context, so the bare `$1 IS TRUE` reports no type at
-- all — measured, and equally for `NOT $1`, `$1 IS NULL` and `$1 AND true`, so it is
-- a pre-existing gap in that walk rather than anything this node does. What the cast
-- leaves testable is the half that *is* this node's: the eval-time operand gate
-- accepting a value the plan-time gate deferred on (an unbound `$N` is exactly the
-- `None` that gate answers), in both the projection and `WHERE`.
-- @setup
CREATE TABLE tb (id INT, b BOOL);
INSERT INTO tb VALUES (1, true), (2, false), (3, NULL);
-- @param $1 bool true
-- @test
SELECT id, $1::bool IS TRUE AS param_test, $1::bool IS NOT UNKNOWN AS never_unknown
FROM tb WHERE $1::bool IS TRUE ORDER BY id;
