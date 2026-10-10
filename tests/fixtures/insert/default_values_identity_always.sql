-- A `GENERATED ALWAYS` identity column draws its sequence from the row-less source:
-- the widened row is `Expr::Default` in every position, which is the one spelling the
-- `428C9` guard permits for such a column, so both inserts land and the counter
-- advances.

-- @setup
CREATE TABLE dv_ida (id INT GENERATED ALWAYS AS IDENTITY, v TEXT);
INSERT INTO dv_ida DEFAULT VALUES;
INSERT INTO dv_ida DEFAULT VALUES;

-- @test
SELECT id, v FROM dv_ida ORDER BY id;
