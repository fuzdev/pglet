-- `information_schema.check_constraints.check_clause` carries the same renderer's text,
-- so it carried the same re-nesting. The one renderer feeds four surfaces and each is
-- pinned separately, because a fix applied at any one reader would leave the other three.
-- @no-declared-types: both columns this test selects are **string** ones, and every
-- string column of an `information_schema` view is a domain on 18.6 — `sql_identifier`
-- (over `name`, 19) or `character_data` (over `varchar`, 1043) — where pglet declares the
-- base `text` (25). The view's *numeric* columns are not in that class and agree: the
-- `cardinal_number` ones (`ordinal_position`, `datetime_precision`, …) declare `int4`
-- (23) on both engines. That divergence is the view layer's, not this fixture's subject,
-- which is the rendered text in the cell.
-- @setup
CREATE TABLE bchain (
  a INT, b INT, c INT, d INT,
  f BOOL DEFAULT (1 > 0 OR 2 > 0 OR 3 > 0),
  CONSTRAINT bx_and3 CHECK (a > 0 AND b > 0 AND c > 0),
  CONSTRAINT bx_and4 CHECK (a > 1 AND b > 1 AND c > 1 AND d > 1),
  CONSTRAINT bx_or3 CHECK (a > 2 OR b > 2 OR c > 2),
  CONSTRAINT bx_and2 CHECK (a > 3 AND b > 3),
  CONSTRAINT bx_parens_left CHECK ((a > 4 AND b > 4) AND c > 4),
  CONSTRAINT bx_parens_right CHECK (a > 5 AND (b > 5 AND c > 5)),
  CONSTRAINT bx_or_parens_right CHECK (a > 6 OR (b > 6 OR c > 6)),
  CONSTRAINT bx_and_then_or CHECK ((a > 7 AND b > 7) OR c > 7),
  CONSTRAINT bx_or_under_and CHECK (a > 8 AND (b > 8 OR c > 8)),
  CONSTRAINT bx_and3_with_or CHECK (a > 9 AND b > 9 AND (c > 9 OR d > 9)),
  CONSTRAINT bx_not_and CHECK (NOT (a > 10 AND b > 10)),
  CONSTRAINT bx_and_not_or CHECK (a > 11 AND NOT (b > 11 OR c > 11)),
  CONSTRAINT bx_arith CHECK (a + b + c > 12),
  CONSTRAINT bx_arith_right CHECK (a - (b - c) > 13)
);
CREATE INDEX bchain_and3_idx ON bchain (a) WHERE (a > 0 AND b > 0 AND c > 0);
CREATE INDEX bchain_or3_idx ON bchain (b) WHERE (a > 0 OR b > 0 OR c > 0);

-- @test
SELECT constraint_name, check_clause
FROM information_schema.check_constraints
WHERE constraint_name LIKE 'bx\_%' ORDER BY constraint_name
