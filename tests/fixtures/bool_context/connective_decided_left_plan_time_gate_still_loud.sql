-- A short-circuit skips evaluation, not planning: an operator with no signature for its
-- operand types is refused at plan time whatever the left operand answers.
-- @setup
CREATE TABLE sc (id INT, tx TEXT);
INSERT INTO sc VALUES (1, 'x');
-- @expect-error: operator does not exist: text < integer
-- @test
SELECT count(*) FROM sc WHERE id = 99 AND tx < 5;
