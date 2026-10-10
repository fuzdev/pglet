-- `ADD COLUMN IF NOT EXISTS` of a serial column the table lacks declares it `NOT NULL` as
-- the plain spelling does.
-- @expect-error: null value in a NOT NULL column (23502)
-- @setup
CREATE TABLE acs (v TEXT);
ALTER TABLE acs ADD COLUMN IF NOT EXISTS id SERIAL;
-- @test
INSERT INTO acs (id, v) VALUES (NULL, 'n');
