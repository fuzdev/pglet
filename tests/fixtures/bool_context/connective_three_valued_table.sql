-- `AND` / `OR` over every pair of `true`, `false` and NULL, read from columns so nothing is
-- folded: `NULL AND false` is `false` and `NULL OR true` is `true` — a NULL left operand
-- leaves the answer to the right one.
-- @setup
CREATE TABLE tv (v BOOL);
INSERT INTO tv VALUES (true), (false), (NULL);
-- @test
SELECT x.v AS l, y.v AS r, (x.v AND y.v) AS a, (x.v OR y.v) AS o
FROM tv x, tv y ORDER BY x.v NULLS LAST, y.v NULLS LAST;
