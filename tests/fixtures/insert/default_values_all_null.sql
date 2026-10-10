-- `INSERT … DEFAULT VALUES` over a table where **no** column carries a default: every
-- column takes SQL NULL and the statement succeeds. Measured on 18.6 — the row-less
-- source is not an error on its own, which is the half a reader expects to be `23502`
-- and is not; the not-null case is `default_values_not_null_no_default.sql`.

-- @setup
CREATE TABLE dv_all_null (id INT, v TEXT, n NUMERIC);
INSERT INTO dv_all_null DEFAULT VALUES;

-- @test
SELECT id, v, n FROM dv_all_null;
