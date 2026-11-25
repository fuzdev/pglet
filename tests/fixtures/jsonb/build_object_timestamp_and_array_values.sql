-- `jsonb_build_object`'s values convert as `to_jsonb` converts them: timestamps in ISO
-- 8601 (a `timestamptz` with `+00:00`), arrays as JSON arrays.
-- @setup
CREATE TABLE tj_bo (c TIMESTAMPTZ, d TIMESTAMP, g TEXT[]);
INSERT INTO tj_bo VALUES ('2026-01-02 03:04:05+00', '2026-01-02 03:04:05.25', '{a,NULL}');
-- @test
SELECT jsonb_build_object('c', c, 'd', d, 'g', g, 'n', ARRAY[1.5, 2]) FROM tj_bo;
