-- Dropping a column auto-drops the table's own CHECK constraints that reference it
-- (Postgres: an AUTO pg_depend edge, no CASCADE needed). pglet used to leave the
-- constraint behind, so every later write to the table failed 42703 — an
-- accepted-then-unwritable table.
-- @setup
CREATE TABLE dcca (id INT PRIMARY KEY, a INT, b INT, CHECK (a > 0));
INSERT INTO dcca VALUES (1, 5, 5);

-- @profile-lock
-- @test
ALTER TABLE dcca DROP COLUMN a;
INSERT INTO dcca VALUES (2, 7);
SELECT id, b FROM dcca ORDER BY id;
