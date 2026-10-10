-- The other direction of the type-name dispatch: a column named `date` still
-- resolves **bare**, and arithmetic on it is still arithmetic — only an immediately
-- following string literal makes the word a type. Both spellings in one row, so a
-- parser that committed to the type name on the lead word alone reddens here rather
-- than only in the error fixture.
-- @setup
CREATE TABLE tc (id INT, date TEXT, foo TEXT);
INSERT INTO tc VALUES (1, 'colval', 'fooval');
-- @test
SELECT date AS col, DATE '2024-06-15' AS konst, id AS i FROM tc;
