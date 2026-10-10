-- An aggregate nested inside COALESCE in a no-GROUP-BY projection — the fuz spine's
-- orphan-fact summary (`SELECT COUNT(*)::bigint, COALESCE(SUM(size), 0)::bigint`).
-- The aggregate must be computed via the aggregate evaluator even when wrapped in
-- COALESCE (and a CAST); COALESCE returns the SUM when present.
-- @setup
CREATE TABLE fact (hash TEXT, size INT);
INSERT INTO fact VALUES ('a', 10), ('b', 20), ('c', NULL);
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0)::bigint AS total FROM fact;
