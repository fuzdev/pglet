-- A `bytea[]` column takes an array literal's elements by the same rule on a write, and a
-- containment probe reads its literal the same way.
-- @setup
CREATE TABLE bytea_input_a (id INT, bs BYTEA[]);
INSERT INTO bytea_input_a VALUES (1, '{abcd,cafe}'), (2, '{"\\xabcd",\\xcafe}'), (3, '{}'), (4, '{" ",NULL}');
UPDATE bytea_input_a SET bs = '{face}' WHERE id = 3;
-- @test
SELECT id, bs, bs = '{abcd,cafe}' AS is_plain, bs @> '{abcd}' AS holds_plain FROM bytea_input_a ORDER BY id;
