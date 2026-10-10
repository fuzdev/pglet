-- The `INSERT … SELECT` polarity of the silent no-FROM bound (see
-- `select/no_from_limit_zero_in_a_delete.sql` for the class). This one **invents** a row
-- rather than removing one: with the bound discarded the no-FROM source answered its row
-- and the insert wrote it, `INSERT 0 1` here against 18.6's `INSERT 0 0`, no error on
-- either engine.
-- @setup
CREATE TABLE lzi (a INT);
INSERT INTO lzi SELECT 7 AS q LIMIT 0;

-- @test
SELECT a FROM lzi ORDER BY a;
