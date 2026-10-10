-- `IF NOT EXISTS` is tested at the top of `transformCreateStmt`, ahead of the whole
-- before-list, so a colliding declared SEQUENCE NAME never gets looked at — the
-- statement is a plain skip. A build that gated the name earlier would error here.
-- @setup
CREATE TABLE idine (a INT);
CREATE SEQUENCE idine_taken;
CREATE TABLE IF NOT EXISTS idine (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idine_taken));
-- @test
SELECT count(*) AS tables_created FROM information_schema.tables
 WHERE table_name = 'idine';
