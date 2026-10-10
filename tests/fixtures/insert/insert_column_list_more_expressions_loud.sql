-- A `VALUES` row longer than its column list — a `DEFAULT` counts as an expression —
-- is `42601 INSERT has more expressions than target columns`.
-- @setup
CREATE TABLE icme (i INT);

-- @test
-- @expect-error: INSERT has more expressions than target columns (42601)
-- @expect-message
INSERT INTO icme (i) VALUES (DEFAULT, 1);
