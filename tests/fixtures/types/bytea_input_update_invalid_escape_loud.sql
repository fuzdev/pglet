-- @setup
CREATE TABLE bytea_input_ul (id INT, b BYTEA);
INSERT INTO bytea_input_ul VALUES (1, '\x00');
-- @test
-- @expect-error: an UPDATE's SET string holding an invalid escape is 22P02
UPDATE bytea_input_ul SET b = 'a\b' WHERE id = 1;
