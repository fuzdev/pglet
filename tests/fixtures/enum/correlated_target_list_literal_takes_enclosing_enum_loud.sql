-- A literal compared with a scalar subquery whose target list names an enclosing enum
-- column, over an inner table carrying a same-named `text` column. The literal is read
-- as the column the reference binds to — the outer `mood` — so a non-member is 18.6's
-- `22P02`; read through the inner `text` column it was never converted and answered.
-- @setup
CREATE TYPE ctm_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ctm_te (m ctm_mood, k INT);
CREATE TABLE ctm_tt (m TEXT, k TEXT);
INSERT INTO ctm_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO ctm_tt VALUES ('x', 'y');
-- @expect-error: a non-member literal beside an enclosing enum column — 22P02
-- @test
SELECT k, (SELECT o.m FROM ctm_tt b) > 'zzz' FROM ctm_te o ORDER BY k;
