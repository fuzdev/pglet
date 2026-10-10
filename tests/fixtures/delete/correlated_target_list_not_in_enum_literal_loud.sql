-- A `DELETE … WHERE 'zzz' NOT IN (SELECT te.m FROM tt)`, the sub-plan projecting the
-- target's enum column over an inner table with a same-named `text` column. 18.6
-- converts the literal to the enum and refuses `22P02`, deleting nothing; compared as
-- `text` it matched no member and deleted every row.
-- @setup
CREATE TYPE ctn_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ctn_te (m ctn_mood, k INT);
CREATE TABLE ctn_tt (m TEXT, k TEXT);
INSERT INTO ctn_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO ctn_tt VALUES ('x', 'y');
-- @expect-error: a non-member literal beside the target's enum column — 22P02
-- @test
DELETE FROM ctn_te WHERE 'zzz' NOT IN (SELECT ctn_te.m FROM ctn_tt) RETURNING k;
