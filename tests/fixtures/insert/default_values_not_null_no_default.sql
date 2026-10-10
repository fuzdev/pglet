-- A `NOT NULL` column with no default takes NULL from the row-less source and
-- violates: `23502`, raised at execution on both engines. The setup cannot fail, so
-- the assertion is the test statement's own.

-- @expect-error: NOT NULL with no default violates (23502)
-- @setup
CREATE TABLE dv_nn (id INT NOT NULL, v TEXT);

-- @test
INSERT INTO dv_nn DEFAULT VALUES;
