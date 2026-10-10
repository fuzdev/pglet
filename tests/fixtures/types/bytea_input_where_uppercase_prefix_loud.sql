-- @setup
CREATE TABLE bytea_input_wl (id INT, b BYTEA);
INSERT INTO bytea_input_wl VALUES (1, '\x61');
-- @test
-- @expect-error: a literal compared with a bytea column is read as bytea, 22P02
SELECT id FROM bytea_input_wl WHERE b = '\X61';
