-- The control that identifies the duplicate-column gate as a test on the **folded**
-- name rather than on the written spelling: `"a"` and `"A"` are two distinct columns
-- on both engines, where an unquoted `A` beside `"a"` is one name twice.
-- @setup
CREATE TABLE ctdf ("a" INT, "A" TEXT);
INSERT INTO ctdf VALUES (1, 'x');
-- @test
SELECT column_name::text AS column_name, ordinal_position
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'ctdf'
ORDER BY ordinal_position;
