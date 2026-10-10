-- A set operation inside a scalar subquery whose left side projects an enclosing enum
-- column, over an inner table carrying a same-named `text` column. The column the
-- reference binds to is the outer `mood`, enum identity included, so the right side's
-- non-member `'zzz'` is 18.6's `22P02`.
-- @setup
CREATE TYPE cto2_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE cto2_te (m cto2_mood, k INT);
CREATE TABLE cto2_tt (m TEXT, k TEXT);
INSERT INTO cto2_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO cto2_tt VALUES ('x', 'y');
-- @expect-error: a non-member side unified with an enclosing enum column — 22P02
-- @test
SELECT k, (SELECT o.m FROM cto2_tt UNION SELECT 'zzz' LIMIT 1) AS v FROM cto2_te o ORDER BY k;
