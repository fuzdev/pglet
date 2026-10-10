-- Bound ILIKE pattern: `$1` types text from the column sibling (`Ilike` is in
-- the sibling-inference operator group in `param_types.rs`).
-- @setup
CREATE TABLE p_items (id INT, name TEXT);
INSERT INTO p_items VALUES (1, 'Widget'), (2, 'WIDGETRY'), (3, 'gadget');
-- @param $1 text widget%
-- @test
SELECT id FROM p_items WHERE name ILIKE $1 ORDER BY id;
