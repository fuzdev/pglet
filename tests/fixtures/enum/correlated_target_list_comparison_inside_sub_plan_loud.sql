-- Control: a comparison written **inside** the sub-plan reads the enclosing enum column
-- at its own level, so a non-member literal is `22P02` on both engines.
-- @setup
CREATE TYPE ctk_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ctk_te (m ctk_mood, k INT);
CREATE TABLE ctk_tt (m TEXT, k TEXT);
INSERT INTO ctk_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO ctk_tt VALUES ('x', 'y');
-- @expect-error: a non-member literal compared inside the sub-plan — 22P02
-- @test
SELECT k, (SELECT o.m > 'zzz' FROM ctk_tt b) FROM ctk_te o ORDER BY k;
