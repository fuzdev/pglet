-- @setup
-- A string literal in an IN list (or nested inside a CASE WHEN) is coerced to the
-- left column's type, exactly as Postgres treats `IN` like a scalar `=`. Before
-- the coercion walker descended into InList/Case, `int_col IN ('5')` matched zero
-- rows (the literal stayed text, with no Int=Text comparison arm).
CREATE TABLE coerce_in (id INT, b BOOL);
INSERT INTO coerce_in VALUES (5, true), (6, false), (7, true);

-- @test
SELECT id,
       (id IN ('5', '7')) AS in_str,
       (b IN ('true')) AS bool_in_str,
       CASE WHEN id IN ('6') THEN 'six' ELSE 'other' END AS nested
FROM coerce_in
WHERE id IN ('5', '6')
ORDER BY id;
