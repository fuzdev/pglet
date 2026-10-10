-- `DEFAULT VALUES` is its own `insert_rest` alternative and that alternative carries
-- **no** column list, so the pairing is a syntax error at the `DEFAULT` token rather
-- than a statement with a meaning. Measured on 18.6: `42601 syntax error at or near
-- "DEFAULT"`.
--
-- The discriminating half is that the error is `42601` and not an arity complaint: a
-- row-less source admitted after a one-column list lowers to zero cells against one
-- target and reports `INSERT has 0 expressions but 1 target columns`, which is loud,
-- differently coded, and blames the wrong thing.

-- @expect-error: DEFAULT VALUES takes no column list (42601)
-- @setup
CREATE TABLE dv_cl (id INT DEFAULT 5, v TEXT);

-- @test
INSERT INTO dv_cl (id) DEFAULT VALUES;
