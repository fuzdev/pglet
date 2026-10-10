-- An equality against an array of another element type has no operator, whether or not
-- the column carries a unique index a probe could reach: `integer[] = bigint[]`.
-- @profile-lock
-- @setup
CREATE TABLE ckx_a (id INT, a INT[] NOT NULL UNIQUE);
INSERT INTO ckx_a (a, id) VALUES ('{1}', 1);
-- @test
-- @expect-error: integer[] = bigint[] has no operator (42883)
SELECT id FROM ckx_a WHERE a = '{1}'::int8[];
