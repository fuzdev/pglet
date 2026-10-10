-- to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') over a column of
-- known fixed timestamps: whole-second, sub-second (truncated, no fraction in
-- the format), and midnight. Output column is aliased. Literal values are fixed
-- so the rendering is deterministic.
-- @setup
CREATE TABLE ts_proj (id INT, ts TIMESTAMPTZ);
INSERT INTO ts_proj (id, ts) VALUES
  (1, '2026-06-19 12:30:45Z'),
  (2, '2026-06-19 12:30:45.123456Z'),
  (3, '2026-01-05 00:00:00Z');
-- @test
SELECT id, to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS iso
  FROM ts_proj ORDER BY id;
