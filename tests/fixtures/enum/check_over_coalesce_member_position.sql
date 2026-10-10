-- The wrapped ordering key decides a **write** too. `CHECK (COALESCE(m, 'sad') > 'ok')`
-- admits exactly the members declared after `ok` — `happy` here — and by label bytes it
-- admitted exactly the opposite set, so the row that lands and the row that is refused
-- swapped places with no error anywhere on the accepted one. The surviving rows are the
-- assertion: `check_constraint_member_position` is the same shape one wrapper less deep.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood, CHECK (COALESCE(m, 'sad') > 'ok'));

-- @test
INSERT INTO t VALUES (1, 'happy');
SELECT id, m::text AS label FROM t ORDER BY id;
