-- The locking clause sits **after** the sort and the bound, so this is the shape that pins
-- the clause order rather than the clause: `SELECT 1 AS q ORDER BY 1 LIMIT 1 FOR NO KEY
-- UPDATE` was `42601` here — the arm having consumed the tail and stopped — against one
-- row on 18.6. `NO KEY UPDATE` is the multi-word lock strength, so it also covers the
-- strength grammar the bare `FOR UPDATE` fixture does not
-- (`select/no_from_for_update.sql`).
-- @test
SELECT 1 AS q ORDER BY 1 LIMIT 1 FOR NO KEY UPDATE;
