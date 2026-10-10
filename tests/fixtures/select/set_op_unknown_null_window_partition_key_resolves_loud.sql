-- A window key `NULL` equal to a `NULL` entry resolves that entry to `text` too, and the
-- constant on the left side is resolved the same way.
-- @setup
CREATE TABLE wnk (a INT);
INSERT INTO wnk VALUES (7);
-- @expect-error: UNION types text and integer cannot be matched
-- @test
SELECT NULL, row_number() OVER (PARTITION BY NULL) FROM wnk UNION ALL SELECT 1, 1;
