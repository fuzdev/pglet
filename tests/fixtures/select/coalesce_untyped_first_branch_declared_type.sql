-- `COALESCE(NULL, <computed>)` — the shape where the planner's coercion pass does
-- **not** rewrite the branches, so both declared-type walks reach the raw `Coalesce`
-- node and answer differently.
--
-- The mechanism is `select_common_type` answering `Ok(None)`: `concrete_branch_type`
-- sees a bare `NULL` as unknown and a computed expression (a `StringFn`, a `BinaryOp`,
-- an `Extract`, …) as *nothing at all*, so a branch set of only those two kinds has no
-- known type to fold, and `unify_common_type_branches` returns the branches untouched.
-- No `Cast` is inserted, and the two walks then split: the executor walk takes the
-- **first** branch (its unknown-ness is `text`, since it is total) while the planner
-- walk takes the first **inferable** one, which is 18.6's **answer** over a branch set
-- with one known type — not its rule, which folds the common type of every known branch.
-- Over these three spellings the two coincide, so the simple-query `RowDescription` was
-- the wrong leg here; where they part is `EDGES.md` row 54's fold band.
--
-- Measured on 18.6 and on the pre-fix build: `COALESCE(NULL, length('ab'))` declared
-- `integer` (OID 23) on 18.6 and on `Describe`, against `text` (OID 25) on the
-- simple-query `RowDescription`. The value is the same `2` either way, so the split
-- was visible only as the declared type — which is what psql, node-postgres and the
-- wasm adapter type the column by.
--
-- `typed_first` is the polarity control: the same branch set in the other order, where
-- the two walks already agreed at `integer`. An over-correction that decides from the
-- *last* inferable branch reddens it.
-- @test
SELECT COALESCE(NULL, length('ab'))        AS untyped_first, -- int4 (was text)
       COALESCE(length('ab'), NULL)        AS typed_first,   -- int4 — polarity control
       COALESCE(NULL, NULL, length('abc')) AS untyped_twice  -- int4 (was text)
;
