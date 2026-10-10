-- `UPDATE … SET col = DEFAULT` resolves each column's catalog default, like
-- INSERT: a literal default, a SERIAL/nextval default (which advances the
-- sequence — id becomes 3 after the two seeded rows took 1 and 2), and the
-- impure now()/gen_random_uuid() defaults (compared as IS NOT NULL, since their
-- values are nondeterministic). Previously DEFAULT silently kept the old value.
-- @setup
CREATE TABLE d (
  id SERIAL,
  label TEXT DEFAULT 'def',
  created TIMESTAMPTZ DEFAULT now(),
  uid UUID DEFAULT gen_random_uuid(),
  n INT
);
INSERT INTO d (label, n) VALUES ('a', 1);
INSERT INTO d (label, n) VALUES ('b', 2);
-- @test
UPDATE d
SET id = DEFAULT, label = DEFAULT, created = DEFAULT, uid = DEFAULT
WHERE n = 1
RETURNING id, label, (created IS NOT NULL) AS has_created, (uid IS NOT NULL) AS has_uid;
