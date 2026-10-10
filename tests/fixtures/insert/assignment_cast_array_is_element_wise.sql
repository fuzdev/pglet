-- Two arrays coerce **element-wise** (`COERCION_PATH_ARRAYCOERCE` recurses on the
-- element types with the same context), so `integer[] → text[]` assigns — the element
-- pair reaching the string-target rule — and so does `bigint[] → integer[]`, an
-- assignment cast one element down. Neither pair has a `pg_cast` row of its own.
--
-- Its reject twin is `assignment_cast_unknown_array_is_text_loud.sql`.
-- @setup
CREATE TABLE aca (id INT, ta TEXT[], ia INT[]);
-- @test
INSERT INTO aca (id, ta, ia)
VALUES (1, ARRAY[1, 2], ARRAY[3, 4]::BIGINT[])
RETURNING id, ta, ia;
