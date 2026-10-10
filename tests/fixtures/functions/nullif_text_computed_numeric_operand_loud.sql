-- `NULLIF` over a `text` column a `VALUES` list defines and a computed `numeric`
-- argument is `42883`, as on 18.6: the first argument is not cast to the second's
-- type, under which the `text` `1` answered and a `text` that is no number was `22P02`.
-- @setup
CREATE TABLE w (n NUMERIC);
INSERT INTO w VALUES (1.5);
-- @test
-- @expect-error: operator does not exist: text = numeric
-- @expect-message
SELECT nullif(v.x, w.n * 1) FROM (VALUES ('1'), ('2')) v(x), w;
