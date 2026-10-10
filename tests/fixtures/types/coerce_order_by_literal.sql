-- The same unknown-literal resolution inside `ORDER BY`. Sorting by the boolean
-- `(x = '2')` is an integer comparison in Postgres; before the clause-coercion pass
-- covered `ORDER BY`, the literal reached the operand gate as text and raised a
-- spurious `42883`. The trailing `x` makes the order total.
-- @setup
CREATE TABLE gt (x INT, tag TEXT);
INSERT INTO gt VALUES (1, 'a'), (1, 'b'), (2, 'c');
-- @test
SELECT x FROM gt ORDER BY (x = '2'), x;
