-- Strictness sits **ahead of the body**, so it outranks the kind gate: `setval` over a
-- table with a NULL argument is NULL, not 42809. The regclass step still runs first
-- (`setval('<no such relation>', NULL)` is 42P01 — see `setval_missing_relation_beats_argument_error`),
-- which is what makes this three separate positions rather than one.
-- @setup
CREATE TABLE svn4_host (a INT);
-- @test
SELECT setval('svn4_host', NULL);
