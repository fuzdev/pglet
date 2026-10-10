-- A `VALUES` row shorter than its column list is refused — the columns a list names are
-- all assigned — as `42601 INSERT has more target columns than expressions`. Without a
-- column list the same short row takes its defaults.
-- @setup
CREATE TABLE icmt (a INT, b INT);

-- @test
-- @expect-error: INSERT has more target columns than expressions (42601)
-- @expect-message
INSERT INTO icmt (a, b) VALUES (1);
