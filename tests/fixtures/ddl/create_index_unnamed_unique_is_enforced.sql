-- The minted name is the name of a working index: an unnamed `CREATE UNIQUE INDEX`
-- refuses the duplicate a later write brings.
-- @setup
CREATE TABLE ciu8 (a INT, b INT);
CREATE UNIQUE INDEX ON ciu8 (b);
INSERT INTO ciu8 VALUES (1, 10);
-- @test
INSERT INTO ciu8 VALUES (2, 10);
-- @expect-error: duplicate key value violates unique constraint "ciu8_b_idx" (23505)
-- @no-profile-lock: the refusal is the write path's uniqueness probe, which both profiles run; index selection is not involved
