-- The NULL-padding idiom: a side supplies a bare NULL for a column the other side has
-- a real value for. The NULL column adopts the sibling's type (text here), so the
-- union type-checks and yields (1, NULL), (2, NULL), (3, 'x') instead of failing loud.
-- @setup
CREATE TABLE np_a (id INT);
INSERT INTO np_a VALUES (1), (2);
CREATE TABLE np_b (id INT, note TEXT);
INSERT INTO np_b VALUES (3, 'x');
-- @test
SELECT id, NULL AS note FROM np_a
UNION
SELECT id, note FROM np_b
ORDER BY id;
