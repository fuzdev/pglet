-- `IN (SELECT …)` over a sub-plan projecting an enclosing enum column, the inner table
-- carrying a same-named `text` column: the left-hand literal is converted to the outer
-- `mood`, so a non-member is 18.6's `22P02` rather than an empty match.
-- @setup
CREATE TYPE cti_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE cti_te (m cti_mood, k INT);
CREATE TABLE cti_tt (m TEXT, k TEXT);
INSERT INTO cti_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO cti_tt VALUES ('x', 'y');
-- @expect-error: a non-member literal beside an enclosing enum column — 22P02
-- @test
SELECT k FROM cti_te o WHERE 'zzz' IN (SELECT o.m FROM cti_tt b) ORDER BY k;
