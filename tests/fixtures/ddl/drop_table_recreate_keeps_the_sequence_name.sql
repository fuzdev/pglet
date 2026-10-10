-- The observable a leaked sequence actually moves. The re-created table's *first id*
-- does not: the leak made the namer mint a fresh `dtr_t_a_seq1`, which still hands out
-- 1. What moved is the column's **default**, one burned name per drop/create cycle —
-- so after the two cycles below a leak reads `nextval('dtr_t_a_seq2'::regclass)` here.
-- @setup
CREATE TABLE dtr_t (a SERIAL, b INT);
DROP TABLE dtr_t;
CREATE TABLE dtr_t (a SERIAL, b INT);
DROP TABLE dtr_t;
CREATE TABLE dtr_t (a SERIAL, b INT);
-- @test
SELECT column_default::text AS column_default
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'dtr_t' AND column_name = 'a';
