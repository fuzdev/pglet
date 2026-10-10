-- Set-op dedup counts two NULLs as equal (unlike `=`), so the two NULL rows collapse
-- to one. UNION of the table with itself yields the distinct rows {NULL, 1}; ORDER BY
-- ASC places the NULL last (Postgres default).
-- @setup
CREATE TABLE snd (a INT);
INSERT INTO snd VALUES (NULL), (NULL), (1);
-- @test
SELECT a FROM snd
UNION
SELECT a FROM snd
ORDER BY a;
