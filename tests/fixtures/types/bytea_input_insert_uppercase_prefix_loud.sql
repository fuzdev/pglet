-- @setup
CREATE TABLE bytea_input_il (id INT, b BYTEA);
-- @test
-- @expect-error: a VALUES cell holding `\X61` is 22P02
INSERT INTO bytea_input_il VALUES (1, '\X61');
