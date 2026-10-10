-- Control: the set-operation side projects the enclosing alias's enum column over an
-- inner scan of the **same** table. The reference binds outward — the qualifier is the
-- outer alias — and the type it binds to is the same `mood`, so the non-member `'zzz'`
-- is `22P02` on both engines.
-- @setup
CREATE TYPE cto3_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE cto3_te (m cto3_mood, k INT);
INSERT INTO cto3_te VALUES ('happy', 1), ('sad', 2);
-- @expect-error: a non-member side unified with an enclosing enum column — 22P02
-- @test
SELECT k, (SELECT o.m FROM cto3_te UNION SELECT 'zzz' LIMIT 1) AS v FROM cto3_te o ORDER BY k;
