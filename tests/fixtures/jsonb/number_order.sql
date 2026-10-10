-- jsonb ORDER BY over numbers follows Postgres's exact `numeric_cmp`
-- (`compareJsonbScalarValue`), not an f64 total order: `100` and `1e2` fold equal,
-- `1` and `1.00` fold equal, and a 30-digit integer sorts by its true magnitude.
-- Regression guard for the `jsonb_compare_nested` number arm now comparing via
-- exact `PgNumeric::cmp_value` (the f64→PgNumeric change). Equal values tie-break
-- by id.
-- @setup
CREATE TABLE jn (id INT, v JSONB);
INSERT INTO jn VALUES
  (1, '100'), (2, '2'), (3, '1e2'), (4, '0.5'), (5, '-3'),
  (6, '123456789012345678901234567890'), (7, '1.00'), (8, '1');
-- @test
SELECT id, v FROM jn ORDER BY v, id;
