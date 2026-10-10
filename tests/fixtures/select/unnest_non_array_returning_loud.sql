-- The refusal is the expression's, not the projection clause's: `RETURNING` reaches the
-- same one plan-time walk, so a non-array `unnest` there is `42883` before the `INSERT`
-- writes anything. 18.6 agrees, and its refusal is likewise ahead of the write.
-- @setup
CREATE TABLE unnest_na_ret (i INT);
-- @test
-- @expect-error: no unnest(integer) in RETURNING
INSERT INTO unnest_na_ret VALUES (1) RETURNING unnest(i);
