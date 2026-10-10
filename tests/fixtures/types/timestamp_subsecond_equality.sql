-- @test
-- Two literals that round to the same microsecond ARE the same value. pglet
-- compared the untruncated instants, so this answered `f` while both sides
-- rendered one identical string — the equality half of the same defect, and the
-- one a `UNIQUE` timestamp column turns into a spurious `23505`.
SELECT '2026-01-01 00:00:00.1234561'::timestamp
     = '2026-01-01 00:00:00.1234564'::timestamp AS same_microsecond;
