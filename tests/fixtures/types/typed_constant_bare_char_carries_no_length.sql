-- `gram.y` keeps a separate `ConstTypename` "to allow defaulting fixed-length types
-- such as CHAR() and BIT() to an unspecified length … this makes no sense for
-- constructs like CHAR 'hi'", so `ConstCharacter`'s parenless arm clears the
-- `bpchar(1)` that `Typename` installs. A parenless `char` / `character` therefore
-- carries **no** length at a constant position and length **1** at a column or cast
-- one — the three polarities are in one statement here so an over-correction that
-- reaches the `(n)` spelling or the cast reddens the same fixture the accept side does.
-- The `character varying` columns are the other branch of the same grammar arm: it
-- spells no default at either position, so the split is invisible to it.
-- @no-declared-types: pglet folds `char`/`bpchar` onto one `TypeOid::Varchar`, so every
--   `char` column here declares `varchar` (1043) where 18.6 declares `bpchar` (1042) —
--   `EDGES.md` row 95's modelling gap, which this fixture does not close. The waived
--   comparison is the OID (and the column name) and nothing else: `DeclaredColumn` is
--   `{name, type_oid}`, so no length modifier was ever inside it to waive. The values
--   agree.
-- @test
SELECT
  char 'ab' AS bare,
  character 'ab' AS bare_spelled_out,
  CHAR 'abcdefg' AS bare_long,
  char(1) 'ab' AS with_length,
  character(1) 'ab' AS with_length_spelled_out,
  'ab'::char AS cast_position,
  CAST('ab' AS character) AS cast_word_position,
  character varying 'abc' AS varying_bare,
  character varying(2) 'abc' AS varying_with_length,
  length(char 'ab') AS bare_length,
  char 'ab' = 'ab' AS bare_equals_itself;
