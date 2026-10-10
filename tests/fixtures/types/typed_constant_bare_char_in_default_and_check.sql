-- The rule at the two **persisted** expression positions a migration puts a constant
-- in — a column `DEFAULT` and a table `CHECK`. Both are stored as written and re-read,
-- so this pins that the constant keeps its whole literal across the store as well as at
-- evaluation: the `DEFAULT` writes `ab` and the `CHECK` refuses on the whole literal
-- rather than on its first character. The `id = 2` row is the `CHECK`'s accept side.
-- @setup
CREATE TABLE cdc (
  id INT,
  c TEXT DEFAULT char 'ab',
  CHECK (c <> char 'zz')
);
INSERT INTO cdc (id) VALUES (1);
INSERT INTO cdc (id, c) VALUES (2, 'za');
-- @test
SELECT id, c FROM cdc ORDER BY id;
