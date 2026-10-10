-- `DEFAULT statement_timestamp()` is one instant per statement and not per row: the rows
-- of one multi-row `INSERT` share it, and each statement of the block has its own.
-- @setup
CREATE TABLE csd (id INT, src INT, stmt TIMESTAMPTZ DEFAULT statement_timestamp());
INSERT INTO csd (id, src) VALUES (1, 1), (2, 1), (3, 1), (4, 1);
INSERT INTO csd (id, src) VALUES (5, 2), (6, 2), (7, 2), (8, 2);
-- @test
SELECT src, count(*) AS written_rows, count(DISTINCT stmt) AS statement_values,
       min(stmt) > now() AS after_the_transaction_started
FROM csd GROUP BY src ORDER BY src;
