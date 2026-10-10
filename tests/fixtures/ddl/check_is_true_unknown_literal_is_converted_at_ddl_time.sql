-- A stored expression's `unknown` is converted **once**, at DDL time — there is no
-- later chance, and the dump invariant's catalog part requires an accepted catalog to
-- dump to something stock Postgres loads.
-- 18.6 refuses `CHECK ('x' IS TRUE)` with the conversion's `22P02` (and stores the
-- accepting `CHECK ('t' IS TRUE)` folded to `CHECK ((true IS TRUE))`).
-- @test
-- @expect-error: invalid input syntax for type boolean
CREATE TABLE tx (b BOOL, CHECK ('x' IS TRUE));
