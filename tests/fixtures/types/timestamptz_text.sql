-- @setup
CREATE TABLE ev (id INT, expires_at TIMESTAMPTZ);
INSERT INTO ev VALUES
  (1, '2024-06-15T12:30:45.123Z'),
  (2, '2024-01-02T03:04:05Z'),
  (3, '2024-03-10T08:00:00+05:00');
-- @test
-- @text
-- A `timestamptz` value's text wire output carries the UTC `+00` offset. Without
-- it a driver reparses the value in its own local zone and silently shifts the
-- instant (the fuz_app matrix's node-postgres did exactly this). The binary path
-- is offset-correct already, so only this `@text` (simple-query) fixture guards it.
-- The harness pins both sessions to `TimeZone=UTC`, so pglet (always UTC) matches
-- Postgres byte-for-byte. Covers fractional seconds, whole seconds, and a non-UTC
-- input offset normalized to UTC.
SELECT id, expires_at FROM ev ORDER BY id;
