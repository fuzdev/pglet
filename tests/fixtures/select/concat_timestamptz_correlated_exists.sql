-- The offset is not only a rendering detail: a correlated `EXISTS` whose predicate
-- compares two stringified `timestamptz` values only matches when both sides render
-- the same way. With the outer frame typeless, `'v=' || a.tz` (correlated) dropped
-- the `+00` that `'v=' || b.tz` (inner) kept, so every comparison was false and the
-- query returned **zero rows** where Postgres returns all three — a silently wrong
-- result set, not just a shifted string.
-- @setup
CREATE TABLE ctze (id INT, tz TIMESTAMPTZ);
INSERT INTO ctze VALUES
  (1, '2026-07-18 12:30:45+00'),
  (2, '2026-07-19 01:02:03+00'),
  (3, '2026-07-20 23:59:59+00');
-- @test
SELECT id FROM ctze a
WHERE EXISTS (SELECT 1 FROM ctze b WHERE 'v=' || b.tz = 'v=' || a.tz)
ORDER BY id;
