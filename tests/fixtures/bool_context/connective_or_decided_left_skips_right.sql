-- `OR` stops at a left operand that is `true`: over the row `id = 1` the division is never
-- evaluated, so the row is counted where evaluating it would be `22012`.
-- @setup
CREATE TABLE sc (id INT, tx TEXT);
INSERT INTO sc VALUES (1, 'x');
-- @test
SELECT count(*) FROM sc WHERE (id = 1 OR 1/(id-1) = 0);
