-- An added `SMALLSERIAL` column refuses an `UPDATE` that would write a NULL.
-- @expect-error: null value in a NOT NULL column (23502)
-- @setup
CREATE TABLE acs (v TEXT);
INSERT INTO acs VALUES ('a'), ('b');
ALTER TABLE acs ADD COLUMN id SMALLSERIAL;
-- @test
UPDATE acs SET id = NULL WHERE v = 'a';
