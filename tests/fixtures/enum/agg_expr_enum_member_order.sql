-- An aggregate of an enum under `greatest` / `least` and a `CASE` compared to a label
-- orders by member position, as the column itself does: `'ok'` ranks between `'sad'` and
-- `'happy'`, so a byte comparison would answer otherwise.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE em (m mood);
INSERT INTO em VALUES ('happy'), ('sad');
-- @test
SELECT greatest(min(m), 'ok')::text AS g, least(max(m), 'ok')::text AS l,
  CASE WHEN count(*) > 0 THEN min(m) ELSE 'ok' END < 'ok' AS lt,
  max(m) > 'ok' AS gt
FROM em;
