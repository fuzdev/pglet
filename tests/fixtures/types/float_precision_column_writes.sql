-- The selected type is the **column's**, so it decides what the column stores: at an
-- `INSERT`, and again at an `ALTER TABLE … ADD COLUMN`'s **backfill**, which runs over
-- the already-inserted row and lands the `DEFAULT` through the same narrowing.
-- `g`/`h` are the controls — the same literal, whole, under a modifier that selects
-- `float8` and under no modifier at all.
-- @setup
CREATE TABLE fp (id int, f FLOAT(1), g FLOAT(25), h FLOAT);
INSERT INTO fp (id, f, g, h)
  VALUES (1, 0.1234567890123456, 0.1234567890123456, 0.1234567890123456);
ALTER TABLE fp ADD COLUMN k FLOAT(1) DEFAULT 0.1234567890123456;
-- @test
SELECT f, g, h, k FROM fp ORDER BY id;
