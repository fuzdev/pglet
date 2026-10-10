-- A build over a populated `float8` column back-fills every row, so the ordered scan
-- that then reads the index answers them all — it answered none while the back-fill
-- was refused and the index registered anyway.
-- @profile-lock
-- @setup
CREATE TABLE fkb (g INT, f FLOAT8 NOT NULL);
INSERT INTO fkb VALUES (1, 2.5), (2, 1.5), (3, 'NaN'), (4, '-Infinity');
CREATE INDEX fkb_f ON fkb (f);
-- @test
SELECT g, f::text AS shown FROM fkb ORDER BY f;
