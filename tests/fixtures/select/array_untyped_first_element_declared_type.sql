-- `ARRAY[NULL, <computed>]` — the array member of the un-rewritten-branch family
-- (mechanism: `coalesce_untyped_first_branch_declared_type.sql`).
--
-- The array constructor reaches the same split one dimension up: the executor walk
-- takes the first element's type and wraps it, the planner walk takes the first
-- inferable element's. `ARRAY[NULL, 1]` does **not** reach it — a literal `1` is
-- concrete to `concrete_branch_type`, so the coercion pass unifies the branches and
-- both walks read `int4` off the inserted `Cast`. A *computed* element is what leaves
-- the node un-rewritten.
--
-- Measured on 18.6 and on the pre-fix build: `ARRAY[NULL, length('ab')]` declared
-- `integer[]` (OID 1007) on 18.6 and on `Describe`, against `text[]` (OID 1009) on the
-- simple-query `RowDescription`. The rendered value `{NULL,2}` is identical under
-- either declaration, so nothing but the OID moved.
--
-- `typed_first` is the polarity control, and `literal_second` is the control for the
-- masking claim itself — it is the spelling the coercion pass *does* rewrite, so it
-- must stay `integer[]` on both legs no matter what the walkers do.
-- @test
SELECT ARRAY[NULL, length('ab')] AS untyped_first,  -- int4[] (was text[])
       ARRAY[length('ab'), NULL] AS typed_first,    -- int4[] — polarity control
       ARRAY[NULL, 1]            AS literal_second  -- int4[] — the masked spelling
;
