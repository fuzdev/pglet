-- `DROP SEQUENCE a, b` removes both, and the names are free again — the observable is
-- a `CREATE TABLE` of each name, which is `42P07` while the sequence still holds it
-- (`pg_class` is one namespace over tables, indexes and sequences). The counter is
-- deliberately NOT the observable: a `nextval` here would answer differently on the
-- text leg's setup replay, since a sequence counter is non-transactional.
-- @setup
CREATE SEQUENCE dsm1 START WITH 5;
CREATE SEQUENCE dsm2 START WITH 9;

-- @test
DROP SEQUENCE dsm1, dsm2;
CREATE TABLE dsm1 (a INT);
CREATE TABLE dsm2 (a INT);
SELECT count(*)::int AS reclaimed FROM information_schema.tables
WHERE table_schema = 'public' AND (table_name = 'dsm1' OR table_name = 'dsm2');
