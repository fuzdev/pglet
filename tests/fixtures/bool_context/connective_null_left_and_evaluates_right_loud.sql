-- A NULL left operand decides nothing: `NULL AND x` is `false` when `x` is, so the right
-- operand is still evaluated, and its division raises on both engines.
-- @setup
CREATE TABLE sc (id INT, tx TEXT);
INSERT INTO sc VALUES (1, 'x');
-- @expect-error: division by zero
-- @test
SELECT (NULL::bool AND 1/(id-1) = 0) FROM sc;
