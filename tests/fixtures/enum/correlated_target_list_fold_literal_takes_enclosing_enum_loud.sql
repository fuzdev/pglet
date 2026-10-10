-- `COALESCE` over a scalar subquery projecting an enclosing enum column the inner level
-- does not carry: the fold's type is the enum, so a non-member literal branch is
-- 18.6's `22P02`. Typed by its OID alone the subquery was cast to `text`, which erased
-- the enum, and the fold answered.
-- @setup
CREATE TYPE ctc_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ctc_te (m ctc_mood, k INT);
CREATE TABLE ctc_tt (m TEXT, k TEXT);
INSERT INTO ctc_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO ctc_tt VALUES ('x', 'y');
-- @expect-error: a non-member literal branch beside an enclosing enum column — 22P02
-- @test
SELECT k, coalesce((SELECT o.m FROM ctc_tt b), 'zzz') FROM ctc_te o ORDER BY k;
