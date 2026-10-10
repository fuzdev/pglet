-- An ORDER BY key is evaluated in the Sort node's own scope: a key that
-- stringifies a `timestamptz` must see the same `+00`-bearing text the projection
-- does, or the two disagree about the sort order (`'2026-07-18 12:30:45+00'` vs
-- the offset-less spelling collate differently against a text literal bound).
-- @setup
CREATE TABLE ctzo (id INT, a TIMESTAMPTZ);
INSERT INTO ctzo VALUES
  (1, '2026-07-18 12:30:45+00'),
  (2, '2026-07-19 01:02:03+00'),
  (3, '2026-07-17 23:59:59+00');
-- @test
SELECT id, a::text AS tz_cast FROM ctzo ORDER BY 'v=' || a DESC;
