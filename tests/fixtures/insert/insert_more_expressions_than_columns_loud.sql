-- A `VALUES` row longer than the table, with no column list: 18.6's
-- `42601 INSERT has more expressions than target columns`, where pglet answered `22000`.
-- @setup
CREATE TABLE imec (i INT);

-- @test
-- @expect-error: INSERT has more expressions than target columns (42601)
-- @expect-message
INSERT INTO imec VALUES (1, 2);
