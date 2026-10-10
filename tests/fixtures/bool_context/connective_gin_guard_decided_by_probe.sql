-- A GIN-accelerated `OR` lifts its scan-independent disjunct out as a guard and evaluates
-- it once, ahead of every row; the `OR` it came from evaluates it only where the probe
-- disjunct before it does not decide the row. Here the probe decides the one row, so
-- the division is never reached on either build profile, as on 18.6.
-- @profile-lock
-- @setup
CREATE TABLE gz (v INT);
INSERT INTO gz VALUES (0);
CREATE TABLE gg (j JSONB);
CREATE INDEX gg_g ON gg USING gin (j);
INSERT INTO gg VALUES ('{"a": 1}');
-- @test
SELECT count(*) FROM gz WHERE EXISTS (SELECT 1 FROM gg WHERE gg.j @> '{"a": 1}' OR 1/gz.v = 0);
