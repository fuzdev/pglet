-- The multi-row `VALUES` per-column fold is the second caller that folded the common
-- type raw, with the same silent truncation as the set-op path. `42846`, as Postgres
-- reports it for `VALUES`.
-- @test
-- @expect-error: time and timestamp share a category but do not convert (42846)
SELECT * FROM (VALUES ('10:00:00'::TIME), ('2026-01-01 10:00:00'::TIMESTAMP)) q(x);
