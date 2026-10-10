-- `IF` is an unreserved keyword, so it is a table name too: only `IF` followed by
-- `EXISTS` is the clause. Both spellings over a table named `if`.
-- @setup
CREATE TABLE "if" (a INT, b INT);
ALTER TABLE if DROP COLUMN b;
ALTER TABLE IF EXISTS if ADD COLUMN c INT DEFAULT 4;
INSERT INTO "if" (a) VALUES (1);
-- @test
SELECT * FROM "if";
