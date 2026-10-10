-- A parenless `char[]` is an array of `char(1)`, exactly as a parenless scalar
-- `char` is `char(1)` — the element's default cap survives the `[]`. Both engines
-- truncate each element to one character here; pglet does not model bpchar
-- blank-padding, so a value shorter than the cap would diverge and none is used.
-- @no-declared-types: pglet folds `char`/`bpchar` onto one `TypeOid::Varchar`, so the
--   array declares `_varchar` (1015) where 18.6 declares `_bpchar` (1014) — the bpchar
--   modelling gap, not the element-modifier rule this fixture asserts. The values agree.
-- @test
SELECT '{abcdefg,xy}'::char[] AS c;
