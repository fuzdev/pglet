-- pglet doesn't reset a child's FK column to its default, so `ON DELETE SET
-- DEFAULT` fails loud (0A000) rather than silently substituting `NO ACTION`
-- (which instead rejects the parent delete — a different, wrong behavior).
-- Postgres accepts it, so this is a deliberately pglet-stricter case.
-- @expect-error: ON DELETE SET DEFAULT not supported (pglet-stricter than PG)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE sd_parent (id INT PRIMARY KEY);
-- @test
CREATE TABLE sd_child (id INT, pid INT DEFAULT 0 REFERENCES sd_parent(id) ON DELETE SET DEFAULT);
