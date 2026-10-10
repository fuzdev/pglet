-- A serial column added by `ADD COLUMN` is `NOT NULL`: a written NULL is refused.
-- @expect-error: null value in a NOT NULL column (23502)
-- @setup
CREATE TABLE acs (v TEXT);
ALTER TABLE acs ADD COLUMN id SERIAL;
-- @test
INSERT INTO acs (id, v) VALUES (NULL, 'n');
