-- The legal half of `||`: `text || anynonarray` / `anynonarray || text` stringify
-- the non-text side, for every scalar type pglet models. This must keep working —
-- the operand gate only rejects pairs with no text side (or an array involved).
-- @setup
CREATE TABLE cs (i INT, b BOOL, n NUMERIC(5,2), d DATE, t TIME, ts TIMESTAMP);
INSERT INTO cs VALUES (42, true, 3.50, '2026-07-18', '12:30:45', '2026-07-18 12:30:45');
-- @test
SELECT
  'i=' || i        AS int_r,
  b || '!'         AS bool_l,
  'n=' || n        AS numeric_r,
  d || '/' || t    AS date_time,
  'ts=' || ts      AS ts_r,
  'iv=' || INTERVAL '1 mon 2 days' AS interval_r
FROM cs;
