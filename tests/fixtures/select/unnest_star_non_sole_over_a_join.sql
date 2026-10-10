-- A non-sole bare `*` over a join with a two-array `unnest` beside a table of the
-- alias's name expands both sides, each qualified by its own name.
-- @setup
CREATE TABLE uu (q INT);
INSERT INTO uu VALUES (5);
-- @test
SELECT *, 1 FROM uu, unnest(ARRAY[1, 2], ARRAY[3, 4]) AS zz(q, r) ORDER BY 2;
