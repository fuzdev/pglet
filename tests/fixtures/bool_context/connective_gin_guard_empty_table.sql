-- The GIN guard over an empty table: no row reaches the `OR`, so its failing disjunct is
-- never evaluated on either build profile, as on 18.6.
-- @profile-lock
-- @setup
CREATE TABLE gz (v INT);
INSERT INTO gz VALUES (0);
CREATE TABLE gg (j JSONB);
CREATE INDEX gg_g ON gg USING gin (j);
-- @test
SELECT count(*) FROM gz WHERE EXISTS (SELECT 1 FROM gg WHERE gg.j @> '{"a": 1}' OR 1/gz.v = 0);
