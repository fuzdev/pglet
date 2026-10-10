-- The enclosing enum column two sub-plans down: `IN (SELECT (SELECT o.m FROM tt c) FROM
-- t2 b)`. The inner subquery's column is the outer `mood`, and the level above it reads
-- that identity through it, so the left-hand non-member literal is 18.6's `22P02`.
-- @setup
CREATE TYPE ctx_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ctx_te (m ctx_mood, k INT);
CREATE TABLE ctx_tt (m TEXT, k TEXT);
CREATE TABLE ctx_t2 (z TEXT, y TEXT);
INSERT INTO ctx_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO ctx_tt VALUES ('x', 'y');
INSERT INTO ctx_t2 VALUES ('a', 'b');
-- @expect-error: a non-member literal beside an enclosing enum column two levels down — 22P02
-- @test
SELECT k FROM ctx_te o WHERE 'zzz' IN (SELECT (SELECT o.m FROM ctx_tt c) FROM ctx_t2 b) ORDER BY k;
