-- A sequence is a relation 18.6 reads, so the parameter is what it refuses — unlike a
-- name no relation carries, which is `42P01` ahead of the parameter.
-- @setup
CREATE SEQUENCE up_sq;
-- @param $1 text x
-- @test
-- @expect-error: the sequence resolves, and the parameter is left without a type: 42P18
SELECT last_value FROM up_sq WHERE $1 IS NULL;
