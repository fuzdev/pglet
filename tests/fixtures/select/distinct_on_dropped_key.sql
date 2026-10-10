-- The canonical latest-row-per-group idiom in the spelling that does **not** project
-- its key: `dept` orders and groups the rows and never appears in the output, so it
-- is carried as a junk column and trimmed off above the dedup. This was `0A000`.
-- @setup
CREATE TABLE roster (dept TEXT, name TEXT, salary INT);
INSERT INTO roster VALUES ('eng', 'alice', 100), ('eng', 'bob', 150), ('sales', 'dan', 90);
-- @test
SELECT DISTINCT ON (dept) name FROM roster ORDER BY dept, salary DESC;
