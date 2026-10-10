-- The same unification with the literal branch **first**, so a left-side-only read of
-- the pair answers this one and not its sibling. The two spellings together are what
-- separate the rule from either one-sided shortcut.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, x::text AS lbl
FROM (SELECT 0 AS id, 'ok' AS x UNION ALL SELECT id, m FROM t) q
ORDER BY x, id;
