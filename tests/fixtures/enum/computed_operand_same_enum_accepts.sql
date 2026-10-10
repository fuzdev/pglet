-- The other accept polarity: a computed operand whose own type *is* the enum's. The
-- widened reader answers for both sides here, and the pair shares one identity, so it
-- must stay silent — the over-refusal a deeper reader is the risk of.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
SELECT max(m) > min(m) AS aggregates,
       COALESCE(max(m), min(m))::text AS folded,
       max(CASE WHEN id = 1 THEN m ELSE m END) = max(m) AS through_a_case
  FROM t;
