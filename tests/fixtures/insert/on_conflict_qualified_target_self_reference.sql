-- The positive companion to the ambiguity locks: once the bare spelling is rejected,
-- the **qualified** one has to keep working, and both qualifiers have to reach the row
-- they name. `ocq.n + 1` reads the stored 40 (→ 41), so the increment idiom survives
-- the tightening; `EXCLUDED.t` reads the incoming row. A rule that rejected the bare
-- name by declining to resolve anything would pass the two `@expect-error` fixtures
-- and fail here.
-- @setup
CREATE TABLE ocq (id INT PRIMARY KEY, n INT, t TEXT);
INSERT INTO ocq VALUES (1, 40, 'stored');
-- @test
INSERT INTO ocq VALUES (1, 50, 'incoming')
  ON CONFLICT (id) DO UPDATE SET n = ocq.n + 1, t = EXCLUDED.t
  WHERE ocq.n < EXCLUDED.n
  RETURNING id, n, t;
