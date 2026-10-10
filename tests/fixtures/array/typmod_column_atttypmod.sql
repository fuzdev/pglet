-- `pg_attribute.atttypmod` on an array column holds the **element's** encoded
-- modifier — `7` for a `varchar(3)[]` (`3 + VARHDRSZ`), the same value the scalar
-- `varchar(3)` column beside it reports.
-- @setup
CREATE TABLE tmmod (a varchar(3)[], b varchar(3), c varchar[]);
-- @test
SELECT a.attname, a.atttypmod
FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
WHERE c.relname = 'tmmod' AND a.attnum > 0
ORDER BY a.attnum;
