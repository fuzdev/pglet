-- A whole-row cast to `text` is the row's text output: each column's own text output,
-- comma-separated in parentheses, a NULL column empty, and a value quoted where it is
-- empty or holds a quote, a backslash, a parenthesis, a comma or whitespace.
-- @setup
CREATE TABLE wrt (a INT, b TEXT, c TIMESTAMPTZ, d NUMERIC, e BYTEA, f JSONB, g INT[], h BOOLEAN,
                  i FLOAT8, j DATE, k UUID, l INTERVAL, m TEXT, n TEXT[]);
INSERT INTO wrt VALUES
  (1, 'x"y', '2026-01-02 03:04:05.5+00', 1.50, '\xdead', '{"z": 1, "a": [1, 2]}', '{1,NULL}', true,
   'NaN', '2026-01-02', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '1 day 2 hours', '', '{"a b",c}'),
  (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 'Infinity', NULL, NULL, NULL, 'p(q)r,s\', NULL);
-- @test
SELECT wrt::text FROM wrt ORDER BY a;
