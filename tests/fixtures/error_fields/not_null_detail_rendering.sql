-- Every `Failing row` rendering rule in one row: NULL is the bare word `null`, an
-- empty string is nothing at all between the separators, text is raw and unquoted (a
-- value containing `, ` is indistinguishable from a boundary — the oracle's own
-- behaviour), an array is `{…}`, jsonb is canonical, `numeric(4,2)` is typmod-applied
-- and a `timestamptz` carries `+00` (the harness pins the session to UTC).
-- @setup
CREATE TABLE t_mix (
  n INT, s TEXT, e TEXT, arr TEXT[], j JSONB, ts TIMESTAMPTZ, num NUMERIC(4,2), req INT NOT NULL
);

-- @test
-- @expect-error: the trailing NOT NULL column is omitted
-- @expect-message
INSERT INTO t_mix (n, s, e, arr, j, ts, num)
VALUES (NULL, 'q"uo''te,back', '', ARRAY['p, q','r'], '{"z":[1,2]}', '2024-01-01 03:04:05+00', 1.5);
