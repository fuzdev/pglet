-- Control: a name no level binds is still `42703`, judged with the whole chain.
-- @setup
CREATE TABLE ctn2_p (a INT, b INT);
CREATE TABLE ctn2_q (c INT, d INT);
INSERT INTO ctn2_p VALUES (1, 10);
-- @expect-error: a name no level carries — 42703
-- @test
SELECT (SELECT nosuch FROM ctn2_q) FROM ctn2_p;
