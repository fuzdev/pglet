-- An `UPDATE … SET` right-hand side and its `WHERE` over a declared `numeric` `$N`, each
-- beside an `integer` literal.
-- @setup
CREATE TABLE dp_upd (id INT, n NUMERIC);
INSERT INTO dp_upd VALUES (1, 1.5), (2, 2);
-- @param-declared $1 numeric 1.5
-- @test
UPDATE dp_upd SET n = greatest($1, 0) + 1 WHERE n = nullif($1, 0) RETURNING id, n;
