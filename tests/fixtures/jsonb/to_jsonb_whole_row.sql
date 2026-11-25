-- A whole-row reference: `to_jsonb(t)` is the row as a JSON object keyed by column name,
-- each value converted by its column's type, NULL columns as JSON nulls.
-- @setup
CREATE TABLE tj_row (a INT, b TEXT, c TIMESTAMPTZ, d NUMERIC, e BYTEA, f JSONB, g INT[],
                     h BOOLEAN, i FLOAT8, j DATE, k UUID, l INTERVAL, m TIMESTAMP);
INSERT INTO tj_row VALUES
  (1, 'x"y', '2026-01-02 03:04:05.5+00', 1.50, '\xdead', '{"z": 1, "a": [1, 2]}', '{1,NULL}',
   true, 'NaN', '2026-01-02', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '1 day 2 hours',
   '2026-01-02 03:04:05'),
  (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 'Infinity', NULL, NULL, NULL, NULL);
-- @test
SELECT to_jsonb(t)::text FROM tj_row t ORDER BY a;
