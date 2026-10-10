-- An `EXTRACT` field is a `text` `Const`, and 18.6's `get_func_sql_syntax` prints it
-- through `quote_identifier`: a field that is not a plain lowercase identifier, or is a
-- keyword `quote_identifier` quotes, comes back as a quoted identifier with each `"`
-- doubled, and the written case survives inside the quotes. The rule runs over the
-- stored string, so a bare field word — folded by the grammar — and a lowercase string
-- constant both read back bare.
--
-- Members by class: a case the rule quotes (`x01`–`x03`, the third written as a quoted
-- identifier), a keyword of each quoted category — reserved (`x04`, `x05`),
-- `type_func_name` (`x06`), `col_name` (`x07`) — beside an unreserved one that stays bare
-- (`x08`), an embedded quote (`x09`), the empty string (`x10`), a non-ASCII letter
-- (`x11`), a leading digit (`x12`), a space (`x13`), and the bare polarity: a leading
-- underscore (`x14`), a lowercase string constant (`x15`) and bare field words (`x16`,
-- `x17`). The comparisons are against `numeric` constants or `IS NOT NULL`, so no
-- implicit cast enters the text (an `int` constant would, `EDGES.md` row 90).
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
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'xf' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
