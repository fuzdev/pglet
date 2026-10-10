-- An added `BIGSERIAL` column refuses a written NULL, over a table that held rows when it
-- was added.
-- @expect-error: null value in a NOT NULL column (23502)
-- @setup
CREATE TABLE acs (v TEXT);
INSERT INTO acs VALUES ('a');
ALTER TABLE acs ADD COLUMN id BIGSERIAL;
-- @test
INSERT INTO acs (id, v) VALUES (NULL, 'n');
