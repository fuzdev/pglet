-- The polarity that **agrees** at a parenless `char` constant, and the guard against the
-- attractive wrong fix. 18.6 *stores* a `bpchar`'s trailing blanks and strips them per
-- reader (`varchar.c`'s `bcTruelen` at the `bpchar` operators; the `bpchar→text` cast is
-- a separate trim in a separate file), so the raw value a constant sends is byte-identical
-- on both engines — `od -c` on each answers `a b sp sp sp \n`. Rtrimming the literal at
-- the constant position would make the divergent cells agree and make **this** fixture
-- red; reproducing 18.6 needs the blank-insensitive comparator, which is `EDGES.md`
-- row 95's deferred store-magic work. The second column is the same guard from the other
-- side: an rtrimmed constant would no longer equal a text literal carrying the blanks.
-- The cells that do diverge at this position — `char 'ab ' = char 'ab'`, `length`,
-- `||`, `::text`, a `CHECK` over a blank-carrying constant — have no fixture, a faithful
-- one being red because the engine is wrong; row 95 records them.
-- @no-declared-types: pglet folds `char`/`bpchar` onto one `TypeOid::Varchar`, so the
--   constant declares `varchar` (1043) where 18.6 declares `bpchar` (1042) — `EDGES.md`
--   row 95's modelling gap, which this fixture does not close. The values agree.
-- @test
SELECT
  char 'ab   ' AS blanks_kept,
  char 'ab   ' = 'ab   ' AS equals_the_same_blanks;
