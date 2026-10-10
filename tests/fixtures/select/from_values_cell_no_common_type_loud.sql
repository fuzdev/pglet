-- A FROM-clause `VALUES` folded a per-column common type across its rows but never
-- checked a **cell** on its own, so an argument set with no common type at all resolved
-- to whatever the fold produced and the row was returned. Postgres transforms every
-- `VALUES` expression before it calls `select_common_type` over the columns; the
-- `INSERT … VALUES` list already did, and this list is the same thing one clause over.
-- @expect-error: COALESCE branches have no common type in a FROM-clause VALUES (42804)
-- @test
SELECT * FROM (VALUES (COALESCE(1, 'x'::text))) v(c);
