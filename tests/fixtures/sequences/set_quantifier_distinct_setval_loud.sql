-- `setval`'s spelling of the same write: it moves the counter to an arbitrary value
-- rather than stepping it, so the divergence a dropped modifier left was larger than one
-- id. 42809 on both.
-- @setup
CREATE SEQUENCE sq_sd;
-- @test
-- @expect-error: DISTINCT specified, but setval is not an aggregate function
SELECT setval(DISTINCT 'sq_sd', 50);
