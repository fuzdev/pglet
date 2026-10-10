-- `CASE` whose first `THEN` is a bare `NULL` and whose other branch is computed — the
-- `CASE` member of the un-rewritten-branch family (see
-- `coalesce_untyped_first_branch_declared_type.sql` for the mechanism: every branch is
-- invisible to `concrete_branch_type`, so `select_common_type` answers `Ok(None)` and
-- the branches reach both walks un-cast).
--
-- `CASE` reaches it by **two** spellings, and they are different arms of the executor
-- walk: with an `ELSE` the walk reads the first `WHEN` result and never looks further;
-- with two `WHEN`s and no `ELSE` it reads the first `WHEN` result and never reaches the
-- second. The planner walk chains every `WHEN` result and the `ELSE` and takes the
-- first inferable one, which is 18.6's **answer** over a branch set with one known type
-- — not its rule, which folds the common type of every known branch. Over all three
-- **projected columns** below — those two spellings plus the control — the answer and
-- the rule coincide; where they part is `EDGES.md` row 54's fold band.
--
-- Measured on 18.6 and on the pre-fix build: both untyped-first spellings declared
-- `integer` (OID 23) on 18.6 and on `Describe` against `text` (OID 25) on the
-- simple-query `RowDescription`. `else_typed_first` is the polarity control — the same
-- branch set with the typed result leading, where the two walks already agreed.
-- @test
SELECT CASE WHEN true THEN NULL ELSE length('ab') END              AS else_untyped_first,
       CASE WHEN false THEN NULL WHEN true THEN length('ab') END   AS when_untyped_first,
       CASE WHEN true THEN length('ab') ELSE NULL END              AS else_typed_first;
