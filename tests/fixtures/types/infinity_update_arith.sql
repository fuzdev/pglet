-- A stored infinity read back through arithmetic in an `UPDATE` stays infinite.
-- @setup
CREATE TABLE infup (d DATE, ts TIMESTAMPTZ);
INSERT INTO infup VALUES ('infinity', '-infinity');
UPDATE infup SET d = d + 30, ts = ts + interval '1 day';
-- @test
SELECT d, ts FROM infup;
