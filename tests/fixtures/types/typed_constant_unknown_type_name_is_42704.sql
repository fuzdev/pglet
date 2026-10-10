-- An identifier immediately followed by a string literal is a **type** name, never a
-- column reference — even when a column of that name is in scope. 18.6 answers
-- `42704 type "foo" does not exist` here, not `42703`, and that is what makes the
-- one-token-lookahead dispatch safe to commit to. The companion value fixture
-- `typed_constant_does_not_shadow_a_column_of_the_same_name.sql` runs the other
-- direction over the same table.
-- @setup
CREATE TABLE tc (id INT, date TEXT, foo TEXT);
INSERT INTO tc VALUES (1, 'colval', 'fooval');
-- @test
-- @expect-error: type "foo" does not exist
SELECT foo 'x' FROM tc;
