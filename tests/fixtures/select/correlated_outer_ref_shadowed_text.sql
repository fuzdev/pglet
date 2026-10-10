-- A correlated reference whose column name is **shadowed** by a column of the inner
-- subquery's own relation. `cot_o.x` is text and the inner `i.x` is integer; the
-- bare-name coercion map read `cot_o.x` as the inner `x` and cast `'5'` to integer,
-- leaving `text = integer` for the eval gate to reject (`42883`) on a query Postgres
-- runs. A qualifier naming no relation of the inner scope now declines to coerce, so
-- the text literal compares as text.
-- @setup
CREATE TABLE cot_o (id INT, x TEXT);
CREATE TABLE cot_i (n INT, x INT);
INSERT INTO cot_o VALUES (1, '5'), (2, '6');
INSERT INTO cot_i VALUES (1, 9), (2, 9);
-- @test
SELECT id, x FROM cot_o WHERE EXISTS (SELECT 1 FROM cot_i i WHERE i.n = cot_o.id AND cot_o.x = '5');
