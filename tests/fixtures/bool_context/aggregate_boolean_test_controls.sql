-- The accept-side **control** for a grouping query. `IS [NOT] TRUE` was already correct
-- as a plan-time/eval-time pair before the boolean-context gate and is the model the rest of the
-- boolean contexts were generalized from, so it has to keep answering: `'t'` is
-- TRUE over an aggregate and over a column alike.
-- @setup
CREATE TABLE bcag5 (b TEXT);
INSERT INTO bcag5 VALUES ('x'), ('y');
-- @test
SELECT b, ('t' IS TRUE) AS scalar_leg FROM bcag5 GROUP BY b HAVING 't' IS TRUE
  ORDER BY b;
