-- The control that identifies the row-expanding projection rather than the clause as what
-- made the discarded `DISTINCT` wrong: over the one-row `Plan::Values` the arm was written
-- for, the elision was sound and the answer never differed. The node is built here anyway
-- — a shape test would be a second rule to keep in step with `plan_select_no_from`'s — and
-- this pins that building it changes nothing.
-- @test
SELECT DISTINCT 1 AS q;
