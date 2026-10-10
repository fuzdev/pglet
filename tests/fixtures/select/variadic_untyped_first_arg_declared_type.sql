-- `greatest`/`least`/`nullif` with a bare `NULL` first and a computed argument second —
-- the variadic member of the un-rewritten-branch family (mechanism:
-- `coalesce_untyped_first_branch_declared_type.sql`).
--
-- These three share one IR node and **not** one result rule — `greatest`/`least` fold a
-- common type, `nullif` resolves an `=` operator — so they are listed together to say
-- that what splits the two walks is the *operand* policy and not either result rule.
-- The executor walk wraps every argument in `Some`, so an un-inferable one arrives at
-- the shared rule as `Some(text)` and joins the fold, which then has no common type and
-- falls back to argument one; the planner walk hands the same rule a real `None`, which
-- the fold skips.
--
-- Measured on 18.6 and on the pre-fix build: each of the three declared `integer`
-- (OID 23) on 18.6 and on `Describe`, against `text` (OID 25) on the simple-query
-- `RowDescription`. `greatest_typed_first` is the polarity control.
-- @test
SELECT greatest(NULL, length('ab'))  AS greatest_untyped_first, -- int4 (was text)
       least(NULL, length('ab'))     AS least_untyped_first,    -- int4 (was text)
       nullif(NULL, length('ab'))    AS nullif_untyped_first,   -- int4 (was text)
       greatest(length('ab'), NULL)  AS greatest_typed_first    -- int4 — polarity control
;
