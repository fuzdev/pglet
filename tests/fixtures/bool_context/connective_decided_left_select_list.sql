-- The select-list and `CASE`-condition spellings of the short-circuit: each connective's
-- left operand decides it over the row `id = 1`, so no division is evaluated.
-- @setup
CREATE TABLE sc (id INT, tx TEXT);
INSERT INTO sc VALUES (1, 'x');
-- @test
SELECT (id = 99 AND 1/(id-1) = 0) AS a, (id = 1 OR 1/(id-1) = 0) AS o,
  CASE WHEN id = 99 AND 1/(id-1) = 0 THEN 1 ELSE 0 END AS c FROM sc;
