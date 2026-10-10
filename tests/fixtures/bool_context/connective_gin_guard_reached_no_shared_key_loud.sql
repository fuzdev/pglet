-- The GIN guard reached by a row that shares **no** key with the probe: the index returns
-- no candidate for it, so only a failing guard's handing every row to the recheck `Filter`
-- reaches the division — a guard skipped as `false` would drop the row and count `0` on
-- the accelerated profile alone. (`connective_gin_guard_reached_loud`'s row shares the
-- key `a`, so it is a lossy candidate either way and cannot see that.)
-- @profile-lock
-- @setup
CREATE TABLE gz (v INT);
INSERT INTO gz VALUES (0);
CREATE TABLE gg (j JSONB);
CREATE INDEX gg_g ON gg USING gin (j);
INSERT INTO gg VALUES ('{"b": 2}');
-- @expect-error: division by zero
-- @test
SELECT count(*) FROM gz WHERE EXISTS (SELECT 1 FROM gg WHERE gg.j @> '{"a": 1}' OR 1/gz.v = 0);
