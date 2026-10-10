-- A CHECK constraint is evaluated by the write path, and it must see the **same**
-- string the query does: `a::text` on a `timestamptz` renders with the `+00`
-- offset, so `LIKE '%+00'` holds. Evaluated without the declared column types the
-- predicate saw the offset-less spelling and rejected a row Postgres accepts.
-- @setup
CREATE TABLE ckctz (a TIMESTAMPTZ, CHECK (a::text LIKE '%+00'));
INSERT INTO ckctz VALUES ('2026-07-18 12:30:45+00');
-- @test
SELECT a::text AS tz_cast FROM ckctz;
