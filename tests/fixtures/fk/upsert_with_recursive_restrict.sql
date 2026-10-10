-- The `WITH RECURSIVE` spelling of a key-moving upsert reaches the same `RESTRICT` refusal.
-- @profile-lock
-- @setup
CREATE TABLE upwr_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upwr_c (pid INT REFERENCES upwr_p(id) ON UPDATE RESTRICT, x INT);
INSERT INTO upwr_p VALUES (1, 1), (2, 2);
INSERT INTO upwr_c VALUES (1, 10);

-- @test
-- @expect-error: RESTRICT refuses the data-modifying WITH query's rewrite (23001)
WITH RECURSIVE w AS (INSERT INTO upwr_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 RETURNING id) SELECT * FROM w;
