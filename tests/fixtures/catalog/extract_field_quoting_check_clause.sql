-- `information_schema.check_constraints.check_clause` reads the same renderer as
-- `pg_get_constraintdef`, so it carries the same `EXTRACT` field quoting; each surface is
-- pinned on its own because a fix at one reader would leave the others.
-- `extract_field_quoting_constraintdef.sql` names the members.
-- @no-declared-types: both columns this test selects are **string** ones, and every
-- string column of an `information_schema` view is a domain on 18.6 — `sql_identifier`
-- (over `name`, 19) or `character_data` (over `varchar`, 1043) — where pglet declares the
-- base `text` (25). That divergence is the view layer's, not this fixture's subject,
-- which is the rendered text in the cell.
-- @setup
CREATE TABLE xf (
  t TIMESTAMP,
  CONSTRAINT x01 CHECK (EXTRACT('MONTH' FROM t) > 0.0),
  CONSTRAINT x02 CHECK (EXTRACT('Epoch' FROM t) > 0.0),
  CONSTRAINT x03 CHECK (EXTRACT("Hour" FROM t) > 0.0),
  CONSTRAINT x04 CHECK (EXTRACT('select' FROM t) IS NOT NULL),
  CONSTRAINT x05 CHECK (EXTRACT('user' FROM t) IS NOT NULL),
  CONSTRAINT x06 CHECK (EXTRACT('left' FROM t) IS NOT NULL),
  CONSTRAINT x07 CHECK (EXTRACT('between' FROM t) IS NOT NULL),
  CONSTRAINT x08 CHECK (EXTRACT('action' FROM t) IS NOT NULL),
  CONSTRAINT x09 CHECK (EXTRACT('my"x' FROM t) IS NOT NULL),
  CONSTRAINT x10 CHECK (EXTRACT('' FROM t) IS NOT NULL),
  CONSTRAINT x11 CHECK (EXTRACT('é' FROM t) IS NOT NULL),
  CONSTRAINT x12 CHECK (EXTRACT('1x' FROM t) IS NOT NULL),
  CONSTRAINT x13 CHECK (EXTRACT('a b' FROM t) IS NOT NULL),
  CONSTRAINT x14 CHECK (EXTRACT('_a1' FROM t) IS NOT NULL),
  CONSTRAINT x15 CHECK (EXTRACT('year' FROM t) > 0.0),
  CONSTRAINT x16 CHECK (EXTRACT(MONTH FROM t) > 0.0),
  CONSTRAINT x17 CHECK (EXTRACT(dow FROM t) > 0.0)
);
-- @test
SELECT constraint_name, check_clause
FROM information_schema.check_constraints
WHERE constraint_name LIKE 'x%' ORDER BY constraint_name
