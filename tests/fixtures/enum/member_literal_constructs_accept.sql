-- The discriminating half of the membership guard: the same constructs, one `unknown`
-- literal apart. A **member** label is what 18.6's `enum_in` accepts, so every one of
-- these must stay silent — a rule that refused the literal rather than converting it
-- would redden here and nowhere in the `non_member_literal_*` family.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
SELECT id,
       greatest(m, 'ok')::text AS g,
       COALESCE(m, 'ok')::text AS c,
       (m > 'ok') AS gt,
       (m IN ('ok', 'happy')) AS inl,
       nullif(m, 'ok')::text AS nl,
       (CASE WHEN id = 1 THEN m ELSE 'happy' END)::text AS cs
  FROM t ORDER BY id;
