-- The GIN guard reached: the probe disjunct is `false` for the row, so the `OR` evaluates
-- the division and raises on both build profiles.
-- @profile-lock
-- @setup
CREATE TABLE gz (v INT);
INSERT INTO gz VALUES (0);
CREATE TABLE gg (j JSONB);
CREATE INDEX gg_g ON gg USING gin (j);
INSERT INTO gg VALUES ('{"a": 2}');
-- @expect-error: division by zero
-- @test
SELECT count(*) FROM gz WHERE EXISTS (SELECT 1 FROM gg WHERE gg.j @> '{"a": 1}' OR 1/gz.v = 0);
