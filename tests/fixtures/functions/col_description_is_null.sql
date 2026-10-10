-- `col_description(oid, int4)` answers NULL for every column: pglet models `COMMENT ON`
-- as a no-op, so no column carries a description — and NULL is 18.6's own answer for an
-- OID or column number nothing describes. The argument spellings are the ones 18.6
-- resolves to the one overload: an `int4` (a negative one wraps into `oid`), an `int8`
-- inside the `oid` range, and an `int2` in both places. Declared `text` on both legs.
-- @test
SELECT
  col_description(0, 0) AS zero,
  col_description(99999999, 1) AS absent,
  col_description(-1, -1) AS negative,
  col_description(4294967295, 1) AS max_oid,
  col_description(1::int2, 1::int2) AS smallints,
  col_description(1::bigint, 1) AS bigint_oid;
